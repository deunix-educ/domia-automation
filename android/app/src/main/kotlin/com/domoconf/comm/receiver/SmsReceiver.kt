// SPDX-License-Identifier: AGPL-3.0-or-later
package com.domoconf.comm.receiver

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.provider.Telephony
import android.util.Log
import androidx.core.content.ContextCompat
import com.domoconf.comm.service.CommService

class SmsReceiver : BroadcastReceiver() {

    companion object {
        private const val TAG = "SmsReceiver"
    }

    override fun onReceive(context: Context, intent: Intent) {
        if (intent.action != Telephony.Sms.Intents.SMS_RECEIVED_ACTION) return
        Log.d(TAG, "SMS_RECEIVED broadcast: asking the service to read the inbox")
        // The service reads the inbox itself (see CommService.checkNewSms).
        ContextCompat.startForegroundService(
            context,
            Intent(context, CommService::class.java).apply {
                action = CommService.ACTION_CHECK_SMS
            }
        )
    }
}
