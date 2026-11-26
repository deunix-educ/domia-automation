// SPDX-License-Identifier: AGPL-3.0-or-later
package com.domoconf.comm.receiver

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.telephony.TelephonyManager
import android.util.Log
import com.domoconf.comm.service.CommService
import java.time.Instant
import java.time.ZoneOffset
import java.time.format.DateTimeFormatter

/**
 * Receiver for phone call events.
 *
 * Uses PHONE_STATE (READ_PHONE_STATE permission). Outgoing calls come from
 * ACTION_NEW_OUTGOING_CALL (deprecated since API 29, still delivered on most devices).
 */
class CallReceiver : BroadcastReceiver() {

    companion object {
        private const val TAG = "CallReceiver"

        // Previous state, to tell "missed" from "answered"
        private var previousState = TelephonyManager.CALL_STATE_IDLE
        private var callStartTime = 0L
        private var incomingNumber = ""
    }

    @Suppress("DEPRECATION")
    override fun onReceive(context: Context, intent: Intent) {
        when (intent.action) {
            TelephonyManager.ACTION_PHONE_STATE_CHANGED -> _handlePhoneState(context, intent)
            Intent.ACTION_NEW_OUTGOING_CALL             -> _handleOutgoingCall(context, intent)
        }
    }

    private fun _handlePhoneState(context: Context, intent: Intent) {
        val state  = intent.getStringExtra(TelephonyManager.EXTRA_STATE) ?: return
        @Suppress("DEPRECATION")
        val number = intent.getStringExtra(TelephonyManager.EXTRA_INCOMING_NUMBER) ?: incomingNumber

        val now = _nowIso()

        when (state) {
            TelephonyManager.EXTRA_STATE_RINGING -> {
                // Ringing: incoming call
                incomingNumber = number
                callStartTime  = System.currentTimeMillis()
                previousState  = TelephonyManager.CALL_STATE_RINGING
                Log.d(TAG, "Incoming call: $number")
                _sendEvent(context, "ringing", number, now)
            }

            TelephonyManager.EXTRA_STATE_OFFHOOK -> {
                // Picked up
                if (previousState == TelephonyManager.CALL_STATE_RINGING) {
                    callStartTime = System.currentTimeMillis()
                    previousState = TelephonyManager.CALL_STATE_OFFHOOK
                    Log.d(TAG, "Call answered: $number")
                    _sendEvent(context, "answered", number, now)
                }
            }

            TelephonyManager.EXTRA_STATE_IDLE -> {
                // Call over
                val duration = if (callStartTime > 0)
                    ((System.currentTimeMillis() - callStartTime) / 1000).toInt()
                else 0

                val eventState = when (previousState) {
                    TelephonyManager.CALL_STATE_RINGING  -> "missed"    // Never picked up
                    TelephonyManager.CALL_STATE_OFFHOOK  -> "ended"
                    else                                  -> "ended"
                }

                if (previousState != TelephonyManager.CALL_STATE_IDLE) {
                    Log.d(TAG, "Call over ($eventState, ${duration}s): $number")
                    _sendEvent(context, eventState, number, now, duration)
                }

                previousState  = TelephonyManager.CALL_STATE_IDLE
                callStartTime  = 0L
                incomingNumber = ""
            }
        }
    }

    private fun _handleOutgoingCall(context: Context, intent: Intent) {
        val number = intent.getStringExtra(Intent.EXTRA_PHONE_NUMBER) ?: return
        callStartTime = System.currentTimeMillis()
        Log.d(TAG, "Outgoing call: $number")
        _sendEvent(context, "outgoing", number, _nowIso())
    }

    private fun _sendEvent(
        context: Context,
        state: String,
        number: String,
        timestamp: String,
        duration: Int = 0,
    ) {
        val serviceIntent = Intent(context, CommService::class.java).apply {
            action = CommService.ACTION_CALL_EVENT
            putExtra(CommService.EXTRA_CALL_STATE, state)
            putExtra(CommService.EXTRA_PHONE, number)
            putExtra(CommService.EXTRA_TIMESTAMP, timestamp)
            putExtra(CommService.EXTRA_DURATION, duration)
        }
        androidx.core.content.ContextCompat.startForegroundService(context, serviceIntent)
    }

    private fun _nowIso(): String =
        Instant.now()
            .atOffset(ZoneOffset.UTC)
            .format(DateTimeFormatter.ISO_OFFSET_DATE_TIME)
}
