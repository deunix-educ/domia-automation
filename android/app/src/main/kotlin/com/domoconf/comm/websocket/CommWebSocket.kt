// SPDX-License-Identifier: AGPL-3.0-or-later
package com.domoconf.comm.websocket

import android.util.Log
import com.google.gson.Gson
import okhttp3.OkHttpClient
import okhttp3.Request
import okhttp3.Response
import okhttp3.WebSocket
import okhttp3.WebSocketListener
import java.util.concurrent.TimeUnit

/**
 * WebSocket client to a DomiNet server.
 *
 * It connects, sends JSON messages (received SMS, call events, delivery reports), receives the
 * commands of the server (send an SMS) and reconnects by itself after a failure.
 */
class CommWebSocket(
    private val serverUrl: String,          // wss://<host>/ws/android/device/<token>/
    private val listener: CommWebSocketListener,
) {

    companion object {
        private const val TAG              = "CommWebSocket"
        private const val PING_INTERVAL_S  = 30L   // heartbeat every 30 s
        private const val RECONNECT_DELAY_MS = 5_000L
    }

    private val gson   = Gson()
    private val client = OkHttpClient.Builder()
        .pingInterval(PING_INTERVAL_S, TimeUnit.SECONDS)
        .build()

    private var webSocket: WebSocket? = null
    private var shouldReconnect = true

    /** Opens the connection. */
    fun connect() {
        shouldReconnect = true
        _open()
    }

    /** Closes the connection for good (no automatic reconnection). */
    fun disconnect() {
        shouldReconnect = false
        webSocket?.close(1000, "Service stopped")
        webSocket = null
    }

    /** Sends a JSON object to the server. */
    fun send(payload: Map<String, Any?>) {
        val json = gson.toJson(payload)
        val sent = webSocket?.send(json) ?: false
        if (!sent) Log.w(TAG, "Send failed: the WebSocket is not connected")
    }

    // ------------------------------------------------------------------ //
    //  Messages sent to the server                                          //
    // ------------------------------------------------------------------ //

    /** Tells the server about an incoming SMS. */
    fun sendSmsReceived(from: String, body: String, contactName: String, timestamp: String) {
        send(mapOf(
            "type"         to "sms_received",
            "from"         to from,
            "body"         to body,
            "contact_name" to contactName,
            "timestamp"    to timestamp,
        ))
    }

    /** Tells the server about a call event. */
    fun sendCallEvent(
        state: String,          // ringing | answered | ended | missed | outgoing
        number: String,
        contactName: String,
        timestamp: String,
        duration: Int = 0,
    ) {
        send(mapOf(
            "type"         to "call_event",
            "state"        to state,
            "number"       to number,
            "contact_name" to contactName,
            "timestamp"    to timestamp,
            "duration"     to duration,
        ))
    }

    /** Reports that an outgoing SMS was sent (or failed). */
    fun sendSmsAck(messageId: Int, success: Boolean, error: String = "") {
        send(mapOf(
            "type"       to "sms_sent_ack",
            "message_id" to messageId,
            "success"    to success,
            "error"      to error,
        ))
    }

    // ------------------------------------------------------------------ //
    //  Connection                                                           //
    // ------------------------------------------------------------------ //

    private fun _open() {
        val request = Request.Builder().url(serverUrl).build()
        webSocket = client.newWebSocket(request, object : WebSocketListener() {

            override fun onOpen(ws: WebSocket, response: Response) {
                Log.i(TAG, "Connected to $serverUrl")
                listener.onConnected()
            }

            override fun onMessage(ws: WebSocket, text: String) {
                Log.d(TAG, "← $text")
                _dispatch(text)
            }

            override fun onClosing(ws: WebSocket, code: Int, reason: String) {
                ws.close(1000, null)
            }

            override fun onClosed(ws: WebSocket, code: Int, reason: String) {
                Log.i(TAG, "Closed ($code): $reason")
                listener.onDisconnected()
                if (shouldReconnect) _scheduleReconnect()
            }

            override fun onFailure(ws: WebSocket, t: Throwable, response: Response?) {
                Log.e(TAG, "WebSocket error: ${t.message}")
                listener.onDisconnected()
                if (shouldReconnect) _scheduleReconnect()
            }
        })
    }

    private fun _dispatch(json: String) {
        try {
            @Suppress("UNCHECKED_CAST")
            val data = gson.fromJson(json, Map::class.java) as Map<String, Any>
            when (data["type"] as? String) {
                "auth_ok"   -> listener.onAuthOk(data["device_name"] as? String ?: "")
                "send_sms"  -> {
                    val to   = data["to"]   as? String ?: return
                    val body = data["body"] as? String ?: return
                    val id   = (data["message_id"] as? Double)?.toInt() ?: 0
                    listener.onSendSmsCommand(to, body, id)
                }
                "pong"      -> Log.d(TAG, "pong received")
                else        -> Log.d(TAG, "Message ignored: ${data["type"]}")
            }
        } catch (e: Exception) {
            Log.e(TAG, "Cannot parse JSON: ${e.message}")
        }
    }

    private fun _scheduleReconnect() {
        Log.i(TAG, "Reconnecting in ${RECONNECT_DELAY_MS} ms...")
        android.os.Handler(android.os.Looper.getMainLooper()).postDelayed({
            if (shouldReconnect) _open()
        }, RECONNECT_DELAY_MS)
    }
}

/** Callbacks of the client; the service implements them. */
interface CommWebSocketListener {
    fun onConnected()
    fun onDisconnected()
    fun onAuthOk(deviceName: String)
    fun onSendSmsCommand(to: String, body: String, messageId: Int)
}
