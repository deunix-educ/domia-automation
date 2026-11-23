// SPDX-License-Identifier: AGPL-3.0-or-later
//! Live view of a camera: WHEP client (WebRTC-HTTP Egress Protocol) to MediaMTX, in Rust/WASM (R6 lot 4, ADR 0046).
//!
//! 1. `RTCPeerConnection` with a receive-only video transceiver;
//! 2. SDP offer, wait for the end of ICE gathering (2 s at most);
//! 3. `POST application/sdp` to `<base>/<uuid>/whep`; the SDP response gives the session;
//! 4. the track arrives in the `<video>`; `getStats` monitors the decoded frames, and **a frozen session is restarted**;
//! 5. on closing, `DELETE` of the session resource (MediaMTX frees the connection right away).
//!
//! The pure part (when is a session frozen? what to show?) is tested on the machine; the browser part
//! (`web-sys`) only exists in the WASM.

/// Delay without a newly decoded image, after which the session is restarted.
pub const STALL_AFTER_MS: f64 = 8_000.0;
/// Delay given to a new session to decode its first image (waiting for a key frame from the camera).
pub const STARTUP_GRACE_MS: f64 = 15_000.0;
/// Interval for reading the statistics.
pub const STATS_EVERY_MS: i32 = 2_000;

/// What a reading of the video reception statistics says.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct StatsSample {
    pub frames_decoded: f64,
    pub fps: f64,
    pub width: f64,
    pub height: f64,
    /// Network round trip, in milliseconds.
    pub rtt_ms: Option<f64>,
    pub jitter_ms: f64,
    pub packets_lost: f64,
}

/// Is a session healthy?
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Health {
    Healthy,
    /// No decoded image for too long: restart.
    Stalled,
}

/// Monitors the number of decoded frames of a session.
#[derive(Debug, Clone, Copy)]
pub struct StallDetector {
    started_ms: f64,
    last_frames: f64,
    last_progress_ms: f64,
    seen_a_frame: bool,
}

impl StallDetector {
    #[must_use]
    pub fn new(now_ms: f64) -> Self {
        Self {
            started_ms: now_ms,
            last_frames: 0.0,
            last_progress_ms: now_ms,
            seen_a_frame: false,
        }
    }

    /// Records a reading (`frames_decoded` never decreases within a session) and returns the verdict.
    pub fn observe(&mut self, now_ms: f64, frames_decoded: f64) -> Health {
        if frames_decoded > self.last_frames {
            self.last_frames = frames_decoded;
            self.last_progress_ms = now_ms;
            self.seen_a_frame = true;
            return Health::Healthy;
        }
        // Before the first image: grace delay (waiting for the key frame); afterwards: the frozen-video delay.
        let limit = if self.seen_a_frame {
            STALL_AFTER_MS
        } else {
            STARTUP_GRACE_MS
        };
        let since = if self.seen_a_frame {
            now_ms - self.last_progress_ms
        } else {
            now_ms - self.started_ms
        };
        if since >= limit {
            Health::Stalled
        } else {
            Health::Healthy
        }
    }
}

/// Why an attempt failed, in one sentence for the user (the translation key is chosen by the interface).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Failure {
    /// The browser does not decode the cameras' H.264 (WebRTC).
    NoH264,
    /// MediaMTX does not have this stream (yet): camera off, unreachable or path not created yet.
    NoStream,
    /// MediaMTX refused the offer (incompatible codecs...) or answers something else.
    Http(u16),
    /// Other browser error (short message, no secret).
    Browser(String),
}

/// Translates the HTTP code of a WHEP response.
#[must_use]
pub fn failure_for_status(status: u16, h264_supported: bool) -> Failure {
    match status {
        404 => Failure::NoStream,
        // MediaMTX answers 400 `codecs not supported by client` to a browser without H.264.
        400 if !h264_supported => Failure::NoH264,
        other => Failure::Http(other),
    }
}

/// A safe `<video>` identifier name: the public uuid only contains hexadecimal digits, but a same stream may be displayed several
/// times (two widgets): `suffix` tells them apart.
#[must_use]
pub fn video_id(uuid: &str, suffix: u32) -> String {
    let clean: String = uuid.chars().filter(char::is_ascii_alphanumeric).collect();
    format!("live-video-{clean}-{suffix}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_session_that_keeps_decoding_is_healthy() {
        let mut d = StallDetector::new(0.0);
        for (t, frames) in [(2_000.0, 40.0), (4_000.0, 90.0), (60_000.0, 1_500.0)] {
            assert_eq!(d.observe(t, frames), Health::Healthy);
        }
    }

    #[test]
    fn a_new_session_gets_a_grace_period_for_its_first_frame() {
        let mut d = StallDetector::new(0.0);
        assert_eq!(
            d.observe(10_000.0, 0.0),
            Health::Healthy,
            "l'image cle se fait attendre"
        );
        assert_eq!(d.observe(14_999.0, 0.0), Health::Healthy);
        assert_eq!(d.observe(15_000.0, 0.0), Health::Stalled);
    }

    #[test]
    fn a_frozen_video_is_detected_after_the_stall_delay() {
        let mut d = StallDetector::new(0.0);
        assert_eq!(d.observe(2_000.0, 50.0), Health::Healthy);
        assert_eq!(d.observe(9_999.0, 50.0), Health::Healthy);
        assert_eq!(d.observe(10_000.0, 50.0), Health::Stalled);
        // One more image: the timer starts over.
        assert_eq!(d.observe(10_500.0, 51.0), Health::Healthy);
        assert_eq!(d.observe(18_499.0, 51.0), Health::Healthy);
        assert_eq!(d.observe(18_500.0, 51.0), Health::Stalled);
    }

    #[test]
    fn http_statuses_become_failures_a_person_can_act_on() {
        assert_eq!(failure_for_status(404, true), Failure::NoStream);
        assert_eq!(failure_for_status(400, false), Failure::NoH264);
        assert_eq!(failure_for_status(400, true), Failure::Http(400));
        assert_eq!(failure_for_status(500, false), Failure::Http(500));
    }

    #[test]
    fn video_ids_are_safe_and_distinct_per_display() {
        assert_eq!(video_id("ab12", 3), "live-video-ab12-3");
        assert_eq!(video_id("a\"b<c>", 1), "live-video-abc-1");
        assert_ne!(video_id("ab12", 1), video_id("ab12", 2));
    }
}

// ------------------------------------------------------------------ browser

#[cfg(feature = "hydrate")]
pub mod browser {
    use js_sys::{Function, Promise, Reflect};
    use wasm_bindgen::{JsCast, JsValue, closure::Closure};
    use wasm_bindgen_futures::JsFuture;
    use web_sys::{
        HtmlVideoElement, MediaStream, Request, RequestInit, Response, RtcIceGatheringState,
        RtcPeerConnection, RtcPeerConnectionState, RtcRtpReceiver, RtcRtpTransceiverDirection,
        RtcRtpTransceiverInit, RtcSdpType, RtcSessionDescriptionInit, RtcTrackEvent,
    };

    use super::{Failure, StatsSample, failure_for_status};

    #[derive(Debug)]
    pub struct Session {
        pc: RtcPeerConnection,
        /// Address of the session resource (`Location` header of the WHEP response), to close it.
        resource: Option<String>,
    }

    fn js_message(e: &JsValue) -> String {
        e.as_string()
            .or_else(|| {
                Reflect::get(e, &JsValue::from_str("message"))
                    .ok()
                    .and_then(|m| m.as_string())
            })
            .unwrap_or_else(|| "erreur du navigateur".to_string())
    }

    fn browser_failure(e: &JsValue) -> Failure {
        Failure::Browser(js_message(e).chars().take(120).collect())
    }

    /// Pause (ms).
    pub async fn sleep_ms(ms: i32) {
        let promise = Promise::new(&mut |resolve: Function, _| {
            if let Some(w) = web_sys::window() {
                let _ = w.set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, ms);
            }
        });
        let _ = JsFuture::from(promise).await;
    }

    /// Browser monotonic time (ms).
    #[must_use]
    pub fn now_ms() -> f64 {
        web_sys::window()
            .and_then(|w| w.performance())
            .map_or(0.0, |p| p.now())
    }

    /// Can the browser receive H.264 over WebRTC? (Debian's Firefox ESR: no.)
    #[must_use]
    pub fn h264_supported() -> bool {
        let Some(caps) = RtcRtpReceiver::get_capabilities("video") else {
            return false;
        };
        let Ok(codecs) = Reflect::get(&caps, &JsValue::from_str("codecs")) else {
            return false;
        };
        js_sys::Array::from(&codecs).iter().any(|c| {
            Reflect::get(&c, &JsValue::from_str("mimeType"))
                .ok()
                .and_then(|m| m.as_string())
                .is_some_and(|m| m.eq_ignore_ascii_case("video/H264"))
        })
    }

    /// Resolved when ICE gathering is finished, or after `max_ms`.
    async fn ice_gathered(pc: &RtcPeerConnection, max_ms: i32) {
        if pc.ice_gathering_state() == RtcIceGatheringState::Complete {
            return;
        }
        let done = Promise::new(&mut |resolve: Function, _| {
            let pc2 = pc.clone();
            let cb = Closure::<dyn FnMut()>::new(move || {
                if pc2.ice_gathering_state() == RtcIceGatheringState::Complete {
                    let _ = resolve.call0(&JsValue::NULL);
                }
            });
            pc.set_onicegatheringstatechange(Some(cb.as_ref().unchecked_ref()));
            cb.forget();
        });
        let timeout = Promise::new(&mut |resolve: Function, _| {
            if let Some(w) = web_sys::window() {
                let _ = w.set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, max_ms);
            }
        });
        let _ = JsFuture::from(Promise::race(&js_sys::Array::of2(&done, &timeout))).await;
    }

    fn attach(video_id: &str, stream: &MediaStream) {
        if let Some(v) = web_sys::window()
            .and_then(|w| w.document())
            .and_then(|d| d.get_element_by_id(video_id))
            .and_then(|el| el.dyn_into::<HtmlVideoElement>().ok())
        {
            v.set_muted(true);
            v.set_src_object(Some(stream));
            let _ = v.play();
        }
    }

    /// Establishes the WHEP session and attaches the track to the `<video id=video_id>` element.
    ///
    /// # Errors
    /// [`Failure`]: stream missing, offer refused, browser error.
    pub async fn connect(whep_url: &str, video_id: &str) -> Result<Session, Failure> {
        let win = web_sys::window().ok_or_else(|| Failure::Browser("pas de fenetre".into()))?;
        let pc = RtcPeerConnection::new().map_err(|e| browser_failure(&e))?;
        let init = RtcRtpTransceiverInit::new();
        init.set_direction(RtcRtpTransceiverDirection::Recvonly);
        pc.add_transceiver_with_str_and_init("video", &init);

        let id = video_id.to_string();
        let on_track = Closure::<dyn FnMut(RtcTrackEvent)>::new(move |e: RtcTrackEvent| {
            if let Some(stream) = e.streams().get(0).dyn_ref::<MediaStream>() {
                attach(&id, stream);
            }
        });
        pc.set_ontrack(Some(on_track.as_ref().unchecked_ref()));
        on_track.forget();

        let offer: RtcSessionDescriptionInit = JsFuture::from(pc.create_offer())
            .await
            .map_err(|e| browser_failure(&e))?
            .unchecked_into();
        JsFuture::from(pc.set_local_description(&offer))
            .await
            .map_err(|e| browser_failure(&e))?;
        ice_gathered(&pc, 2_000).await;
        let sdp = pc
            .local_description()
            .ok_or_else(|| Failure::Browser("pas de description locale".into()))?
            .sdp();

        let opts = RequestInit::new();
        opts.set_method("POST");
        opts.set_body(&JsValue::from_str(&sdp));
        let headers = web_sys::Headers::new().map_err(|e| browser_failure(&e))?;
        headers
            .set("Content-Type", "application/sdp")
            .map_err(|e| browser_failure(&e))?;
        opts.set_headers(&headers);
        let req =
            Request::new_with_str_and_init(whep_url, &opts).map_err(|e| browser_failure(&e))?;
        let resp: Response = JsFuture::from(win.fetch_with_request(&req))
            .await
            .map_err(|e| browser_failure(&e))?
            .unchecked_into();
        if resp.status() != 201 {
            pc.close();
            return Err(failure_for_status(resp.status(), h264_supported()));
        }
        let resource = resp
            .headers()
            .get("Location")
            .ok()
            .flatten()
            .and_then(|loc| web_sys::Url::new_with_base(&loc, whep_url).ok())
            .map(|u| u.href());
        let answer_sdp = JsFuture::from(resp.text().map_err(|e| browser_failure(&e))?)
            .await
            .map_err(|e| browser_failure(&e))?
            .as_string()
            .unwrap_or_default();
        let answer = RtcSessionDescriptionInit::new(RtcSdpType::Answer);
        answer.set_sdp(&answer_sdp);
        JsFuture::from(pc.set_remote_description(&answer))
            .await
            .map_err(|e| browser_failure(&e))?;
        Ok(Session { pc, resource })
    }

    impl Session {
        /// Is the connection lost (failure or closure)?
        #[must_use]
        pub fn is_down(&self) -> bool {
            matches!(
                self.pc.connection_state(),
                RtcPeerConnectionState::Failed | RtcPeerConnectionState::Closed
            )
        }

        /// Video reception statistics.
        pub async fn stats(&self) -> StatsSample {
            let mut out = StatsSample::default();
            let Ok(report) = JsFuture::from(self.pc.get_stats()).await else {
                return out;
            };
            let map: js_sys::Map = report.unchecked_into();
            let mut rtt: Option<f64> = None;
            map.for_each(&mut |v: JsValue, _k: JsValue| {
                let num = |name: &str| {
                    Reflect::get(&v, &JsValue::from_str(name))
                        .ok()
                        .and_then(|x| x.as_f64())
                };
                let text = |name: &str| {
                    Reflect::get(&v, &JsValue::from_str(name))
                        .ok()
                        .and_then(|x| x.as_string())
                };
                match text("type").as_deref() {
                    Some("inbound-rtp") if text("kind").as_deref() == Some("video") => {
                        out.frames_decoded = num("framesDecoded").unwrap_or(0.0);
                        out.fps = num("framesPerSecond").unwrap_or(0.0);
                        out.width = num("frameWidth").unwrap_or(0.0);
                        out.height = num("frameHeight").unwrap_or(0.0);
                        out.jitter_ms = num("jitter").unwrap_or(0.0) * 1000.0;
                        out.packets_lost = num("packetsLost").unwrap_or(0.0);
                    }
                    Some("candidate-pair") if text("state").as_deref() == Some("succeeded") => {
                        rtt = num("currentRoundTripTime").map(|s| s * 1000.0);
                    }
                    _ => {}
                }
            });
            out.rtt_ms = rtt;
            out
        }

        /// Closes the session: `DELETE` of the WHEP resource (MediaMTX frees it right away), then local closing.
        pub async fn close(self) {
            self.pc.close();
            if let (Some(url), Some(win)) = (self.resource, web_sys::window()) {
                let opts = RequestInit::new();
                opts.set_method("DELETE");
                let _ = Reflect::set(&opts, &JsValue::from_str("keepalive"), &JsValue::TRUE);
                if let Ok(req) = Request::new_with_str_and_init(&url, &opts) {
                    let _ = JsFuture::from(win.fetch_with_request(&req)).await;
                }
            }
        }
    }
}
