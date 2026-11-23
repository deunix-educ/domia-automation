// SPDX-License-Identifier: AGPL-3.0-or-later
//! Weather stations administration: `/admin/weather-stations` (R3 lot 6, ADR 0030). The publisher queries Open-Meteo for the
//! enabled stations and publishes on `meteo/<suffix>`; creating or modifying a station wakes it up right away.

use dominet_proto::{DeleteOutcome, SaveOutcome, WeatherStationDto, WeatherStationInput};
use leptos::prelude::*;
use leptos_fluent::move_tr;

use crate::crud::{CrudResult, CrudTable, FieldKind, FieldSpec, Labels, Row, Values};
use crate::guard::RequireAdmin;

#[cfg(feature = "ssr")]
fn dto(s: &dominet_domain::device::WeatherStation) -> WeatherStationDto {
    WeatherStationDto {
        id: s.id.0,
        name: s.name.clone(),
        latitude: s.latitude.to_string(),
        longitude: s.longitude.to_string(),
        altitude: s.altitude.to_string(),
        timezone: s.timezone.clone(),
        mqtt_topic_suffix: s.mqtt_topic_suffix.clone(),
        refresh_minutes: s.refresh_minutes.to_string(),
        enabled: s.enabled,
        description: s.description.clone(),
    }
}

/// Weather stations, by name (administrators).
///
/// # Errors
/// Refused (403) for a non-administrator; internal error if the database fails.
#[server(name = ListWeatherStations, prefix = "/sf", endpoint = "weather-stations/list")]
pub async fn list_weather_stations() -> Result<Vec<WeatherStationDto>, ServerFnError> {
    use dominet_domain::repo::WeatherRepo;

    use crate::server_fns::{internal, require_admin};

    let state = expect_context::<dominet_server::AppState>();
    require_admin(&state).await?;
    let items = state
        .store
        .weather()
        .list()
        .await
        .map_err(|e| internal("weather station list", &e))?;
    Ok(items.iter().map(dto).collect())
}

/// Creates (`id` absent) or replaces (`id` present) a station; the publisher is woken up.
///
/// # Errors
/// Refused (403) for a non-administrator; internal error if the database fails.
#[server(name = SaveWeatherStation, prefix = "/sf", endpoint = "weather-stations/save", input = server_fn::codec::Json)]
pub async fn save_weather_station(
    input: WeatherStationInput,
) -> Result<SaveOutcome<WeatherStationDto>, ServerFnError> {
    use dominet_domain::device::{WeatherStation, WeatherStationId};
    use dominet_proto::{FieldProblem, ProblemKind};

    use crate::server_fns::{field_problems, internal, require_admin};

    let state = expect_context::<dominet_server::AppState>();
    let admin = require_admin(&state).await?;
    let mut bad = vec![];
    let mut number = |field: &str, raw: &str| -> Option<f64> {
        let parsed = raw
            .trim()
            .replace(',', ".")
            .parse::<f64>()
            .ok()
            .filter(|n| n.is_finite());
        if parsed.is_none() {
            bad.push(FieldProblem {
                field: field.to_string(),
                kind: ProblemKind::Invalid,
            });
        }
        parsed
    };
    let latitude = number("latitude", &input.latitude);
    let longitude = number("longitude", &input.longitude);
    let altitude = number("altitude", &input.altitude);
    let refresh = number("refresh_minutes", &input.refresh_minutes);
    // Everything is checked at once: an unreadable number does not hide the form's other problems.
    let mut draft = WeatherStation::draft(
        input.name.trim(),
        latitude.unwrap_or_default(),
        longitude.unwrap_or_default(),
    );
    // Integers: a fractional value is rounded, out of range it is saturated then refused by the domain.
    #[allow(clippy::cast_possible_truncation)]
    {
        draft.altitude = altitude
            .unwrap_or_default()
            .round()
            .clamp(f64::from(i32::MIN), f64::from(i32::MAX)) as i32;
        draft.refresh_minutes = refresh
            .unwrap_or(10.0)
            .round()
            .clamp(f64::from(i16::MIN), f64::from(i16::MAX)) as i16;
    }
    draft.timezone = input.timezone.trim().to_string();
    draft.mqtt_topic_suffix = input.mqtt_topic_suffix.trim().to_string();
    draft.enabled = input.enabled;
    draft.description = input.description.trim().to_string();
    let unreadable: Vec<String> = bad.iter().map(|p| p.field.clone()).collect();
    bad.extend(field_problems(
        draft
            .problems()
            .into_iter()
            .filter(|(field, _)| !unreadable.iter().any(|u| u == field))
            .collect(),
    ));
    if !bad.is_empty() {
        return Ok(SaveOutcome::Invalid(bad));
    }
    let saved = dominet_server::config_service::save_weather_station(
        &state,
        input.id.map(WeatherStationId),
        &draft,
    )
    .await
    .map_err(|e| internal("saving a weather station", &e))?;
    Ok(match saved {
        SaveOutcome::Saved(s) => {
            tracing::info!(
                admin_id = admin.id.0,
                station_id = s.id.0,
                created = input.id.is_none(),
                "weather station saved"
            );
            SaveOutcome::Saved(dto(&s))
        }
        SaveOutcome::Invalid(p) => SaveOutcome::Invalid(p),
        SaveOutcome::NotFound => SaveOutcome::NotFound,
        SaveOutcome::Blocked(b) => SaveOutcome::Blocked(b),
    })
}

/// Deletes a weather station (the device it fed stays).
///
/// # Errors
/// Refused (403) for a non-administrator; internal error if the database fails.
#[server(name = DeleteWeatherStation, prefix = "/sf", endpoint = "weather-stations/delete")]
pub async fn delete_weather_station(id: i64) -> Result<DeleteOutcome, ServerFnError> {
    use dominet_domain::device::WeatherStationId;

    use crate::server_fns::{internal, require_admin};

    let state = expect_context::<dominet_server::AppState>();
    let admin = require_admin(&state).await?;
    let out = dominet_server::config_service::delete_weather_station(&state, WeatherStationId(id))
        .await
        .map_err(|e| internal("suppression d'une station meteo", &e))?;
    if out == DeleteOutcome::Deleted {
        tracing::info!(
            admin_id = admin.id.0,
            station_id = id,
            "weather station deleted"
        );
    }
    Ok(out)
}

fn row(s: &WeatherStationDto) -> Row {
    Row {
        id: s.id,
        values: Values::from([
            ("name", s.name.clone()),
            ("latitude", s.latitude.clone()),
            ("longitude", s.longitude.clone()),
            ("altitude", s.altitude.clone()),
            ("timezone", s.timezone.clone()),
            ("mqtt_topic_suffix", s.mqtt_topic_suffix.clone()),
            ("refresh_minutes", s.refresh_minutes.clone()),
            ("enabled", s.enabled.to_string()),
            ("description", s.description.clone()),
        ]),
    }
}

fn fields() -> Vec<FieldSpec> {
    use FieldKind as K;
    vec![
        FieldSpec::new("name", tk!("weather-name"), K::Text)
            .required()
            .max(100)
            .column(),
        FieldSpec::new("latitude", tk!("weather-latitude"), K::Text)
            .required()
            .hint(tk!("weather-latitude-hint"))
            .column(),
        FieldSpec::new("longitude", tk!("weather-longitude"), K::Text)
            .required()
            .column(),
        FieldSpec::new(
            "altitude",
            tk!("weather-altitude"),
            K::Number {
                min: -500,
                max: 9000,
            },
        )
        .default_value("0"),
        FieldSpec::new("timezone", tk!("weather-timezone"), K::Text)
            .required()
            .default_value("Europe/Paris")
            .max(50),
        FieldSpec::new("mqtt_topic_suffix", tk!("weather-suffix"), K::Text)
            .required()
            .default_value("meteo_locale")
            .max(100)
            .hint(tk!("weather-suffix-hint"))
            .column(),
        FieldSpec::new(
            "refresh_minutes",
            tk!("weather-refresh"),
            K::Number { min: 1, max: 1440 },
        )
        .default_value("10")
        .column(),
        FieldSpec::new("enabled", tk!("weather-enabled"), K::Checkbox)
            .default_value("true")
            .column(),
        FieldSpec::new("description", tk!("weather-description"), K::Text).max(255),
    ]
}

#[component]
pub fn WeatherStationsPage() -> impl IntoView {
    view! {
        <RequireAdmin>
            <main class="vc-container">
                <WeatherStationsAdmin/>
            </main>
        </RequireAdmin>
    }
}

#[component]
fn WeatherStationsAdmin() -> impl IntoView {
    let version = RwSignal::new(0u32);
    let list = Resource::new(
        move || version.get(),
        |_| async { list_weather_stations().await.map_err(|_| ()) },
    );
    let rows = Signal::derive(move || match list.get() {
        Some(Ok(items)) => items.iter().map(row).collect(),
        _ => vec![],
    });
    let save = Action::new(|(id, v): &(Option<i64>, Values)| {
        let text = |k: &str| v.get(k).cloned().unwrap_or_default();
        let input = WeatherStationInput {
            id: *id,
            name: text("name"),
            latitude: text("latitude"),
            longitude: text("longitude"),
            altitude: text("altitude"),
            timezone: text("timezone"),
            mqtt_topic_suffix: text("mqtt_topic_suffix"),
            refresh_minutes: text("refresh_minutes"),
            enabled: v.get("enabled").is_some_and(|a| a == "true"),
            description: text("description"),
        };
        async move {
            save_weather_station(input)
                .await
                .map_or(CrudResult::Failed, CrudResult::from)
        }
    });
    let remove = Action::new(|id: &i64| {
        let id = *id;
        async move {
            delete_weather_station(id)
                .await
                .map_or(CrudResult::Failed, CrudResult::from)
        }
    });
    view! {
        <Transition fallback=|| ()>
            {move || {
                matches!(list.get(), Some(Err(()))).then(|| {
                    view! { <div class="vc-alert vc-alert--error" role="alert" id="crud-load-error">{move_tr!("crud-load-error")}</div> }
                })
            }}
            <CrudTable
                labels=Labels {
                    title: tk!("weather-title"),
                    add: tk!("weather-add"),
                    edit: tk!("weather-edit"),
                    empty: tk!("weather-empty"),
                }
                fields=fields()
                rows
                save
                remove
                on_change=Callback::new(move |()| version.update(|v| *v += 1))
            />
        </Transition>
    }
}
