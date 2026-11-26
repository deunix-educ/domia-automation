// SPDX-License-Identifier: AGPL-3.0-or-later
package com.domoconf.comm.service

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Intent
import android.database.Cursor
import android.os.Handler
import android.os.IBinder
import android.os.Looper
import android.provider.ContactsContract
import android.telephony.SmsManager
import android.util.Log
import androidx.core.app.NotificationCompat
import com.domoconf.comm.MainActivity
import com.domoconf.comm.R
import com.domoconf.comm.ServerConfig
import com.domoconf.comm.websocket.CommWebSocket
import com.domoconf.comm.websocket.CommWebSocketListener
import com.google.gson.Gson
import com.google.gson.reflect.TypeToken
import java.util.Collections

class CommService : Service() {

    companion object {
        private const val TAG = "CommService"

        var isRunning = false
            private set

        const val ACTION_SMS_RECEIVED   = "com.domoconf.comm.SMS_RECEIVED"
        const val ACTION_CALL_EVENT     = "com.domoconf.comm.CALL_EVENT"
        const val ACTION_CHECK_SMS      = "com.domoconf.comm.CHECK_SMS"
        const val ACTION_RELOAD_SERVERS = "com.domoconf.comm.RELOAD_SERVERS"

        const val EXTRA_PHONE          = "phone"
        const val EXTRA_BODY           = "body"
        const val EXTRA_TIMESTAMP      = "timestamp"
        const val EXTRA_CALL_STATE     = "call_state"
        const val EXTRA_DURATION       = "duration"

        private const val NOTIF_CHANNEL  = "domocomm_service"
        private const val NOTIF_ID       = 1

        /** Ids of the servers whose WebSocket is open now. Written by the network threads. */
        private val online: MutableSet<String> = Collections.synchronizedSet(mutableSetOf())

        /**
         * Set by the screen while it is visible; called on the main thread each time a server
         * connects (true) or disconnects (false). That is what turns the dot of a server green.
         */
        @Volatile
        var statusListener: ((serverId: String, online: Boolean) -> Unit)? = null

        /** A copy of the connected servers, for a screen that comes back in front. */
        fun onlineIds(): Set<String> = synchronized(online) { online.toSet() }

        private val main = Handler(Looper.getMainLooper())

        private fun setOnline(serverId: String, isOnline: Boolean) {
            val changed = if (isOnline) online.add(serverId) else online.remove(serverId)
            if (changed) main.post { statusListener?.invoke(serverId, isOnline) }
        }
    }

    private val gson = Gson()
    private val webSockets = mutableMapOf<String, CommWebSocket>()

    /**
     * Counts the reloads of the connections. A socket closed by a reload reports its closing late: without
     * this, that late report would mark the new connection of the same server as offline.
     */
    @Volatile
    private var generation = 0

    private var lastSmsId = 0L
    private var smsObserver: android.database.ContentObserver? = null

    private val prefs by lazy {
        getSharedPreferences("domocomm_prefs", MODE_PRIVATE)
    }

    // ------------------------------------------------------------------ //
    //  Life cycle                                                           //
    // ------------------------------------------------------------------ //

    override fun onCreate() {
        super.onCreate()
        isRunning = true
        createNotificationChannel()
        Log.i(TAG, "CommService created")
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        startForeground(NOTIF_ID, buildNotification(getString(R.string.notif_connecting)))

        when (intent?.action) {
            ACTION_RELOAD_SERVERS -> {
                reloadConnections()
                if (smsObserver == null) registerSmsObserver()
            }
            ACTION_CHECK_SMS  -> checkNewSms()
            ACTION_CALL_EVENT -> onCallEvent(intent)
            else -> {
                if (webSockets.isEmpty()) {
                    if (!connectToAllServers()) return START_NOT_STICKY
                }
                if (smsObserver == null) registerSmsObserver()
            }
        }

        return START_STICKY
    }

    private fun connectToAllServers(): Boolean {
        val servers = loadServers()
        if (servers.isEmpty()) {
            Log.w(TAG, "No server configured, stopping the service")
            stopSelf()
            return false
        }
        for (cfg in servers) {
            val ws = CommWebSocket(cfg.wsUrl, makeListener(cfg.id, cfg.name, generation))
            webSockets[cfg.id] = ws
            ws.connect()
        }
        updateNotification()
        return true
    }

    private fun closeConnections() {
        generation++
        webSockets.values.forEach { it.disconnect() }
        webSockets.clear()
        // Every server is now offline, tell the screen before forgetting them.
        onlineIds().forEach { setOnline(it, false) }
    }

    private fun reloadConnections() {
        closeConnections()
        connectToAllServers()
    }

    override fun onDestroy() {
        super.onDestroy()
        isRunning = false
        smsObserver?.let { contentResolver.unregisterContentObserver(it) }
        smsObserver = null
        closeConnections()
        Log.i(TAG, "CommService destroyed")
    }

    override fun onBind(intent: Intent?): IBinder? = null

    // ------------------------------------------------------------------ //
    //  One listener per server                                              //
    // ------------------------------------------------------------------ //

    private fun makeListener(serverId: String, serverName: String, gen: Int) = object : CommWebSocketListener {

        override fun onConnected() {
            if (gen != generation) return
            Log.i(TAG, "[$serverName] Connected")
            setOnline(serverId, true)
            updateNotification()
        }

        override fun onDisconnected() {
            if (gen != generation) return
            Log.i(TAG, "[$serverName] Disconnected")
            setOnline(serverId, false)
            updateNotification()
        }

        override fun onAuthOk(deviceName: String) {
            Log.i(TAG, "[$serverName] Authenticated: $deviceName")
        }

        override fun onSendSmsCommand(to: String, body: String, messageId: Int) {
            Log.i(TAG, "[$serverName] Sending an SMS")
            try {
                @Suppress("DEPRECATION")
                val smsManager = SmsManager.getDefault()
                val parts      = smsManager.divideMessage(body)

                if (parts.size == 1) {
                    smsManager.sendTextMessage(to, null, body, null, null)
                } else {
                    smsManager.sendMultipartTextMessage(to, null, parts, null, null)
                }

                webSockets[serverId]?.sendSmsAck(messageId, success = true)
                Log.i(TAG, "[$serverName] SMS sent")
            } catch (e: Exception) {
                Log.e(TAG, "[$serverName] Cannot send the SMS: ${e.message}")
                webSockets[serverId]?.sendSmsAck(messageId, success = false, error = e.message ?: "Unknown error")
            }
        }
    }

    // ------------------------------------------------------------------ //
    //  Incoming events (SMS and calls, from the broadcast receivers)         //
    // ------------------------------------------------------------------ //

    private fun onCallEvent(intent: Intent) {
        val state     = intent.getStringExtra(EXTRA_CALL_STATE) ?: return
        val phone     = intent.getStringExtra(EXTRA_PHONE)      ?: return
        val timestamp = intent.getStringExtra(EXTRA_TIMESTAMP)  ?: return
        val duration  = intent.getIntExtra(EXTRA_DURATION, 0)
        val contact   = lookupContact(phone)

        Log.d(TAG, "-> WS call_event $state")
        broadcastToAll { it.sendCallEvent(state, phone, contact, timestamp, duration) }
    }

    // ------------------------------------------------------------------ //
    //  SMS inbox observer                                                   //
    // ------------------------------------------------------------------ //

    private fun registerSmsObserver() {
        if (androidx.core.content.ContextCompat.checkSelfPermission(
                this, android.Manifest.permission.READ_SMS
            ) != android.content.pm.PackageManager.PERMISSION_GRANTED
        ) {
            Log.w(TAG, "READ_SMS not granted, the SMS observer is not started")
            return
        }
        if (lastSmsId == 0L) {
            contentResolver.query(
                android.net.Uri.parse("content://sms"),
                arrayOf("_id"), null, null, "_id DESC"
            )?.use { if (it.moveToFirst()) lastSmsId = it.getLong(0) }
        }
        smsObserver = object : android.database.ContentObserver(Handler(Looper.getMainLooper())) {
            override fun onChange(selfChange: Boolean) = checkNewSms()
        }
        contentResolver.registerContentObserver(
            android.net.Uri.parse("content://sms"),
            true,
            smsObserver!!
        )
        Log.i(TAG, "SMS observer started (lastId=$lastSmsId)")
    }

    private fun checkNewSms() {
        Thread {
            try {
                contentResolver.query(
                    android.net.Uri.parse("content://sms/inbox"),
                    arrayOf("_id", "address", "body", "date"),
                    "_id > ?",
                    arrayOf(lastSmsId.toString()),
                    "_id ASC"
                )?.use { cursor ->
                    while (cursor.moveToNext()) {
                        val id      = cursor.getLong(cursor.getColumnIndexOrThrow("_id"))
                        val address = cursor.getString(cursor.getColumnIndexOrThrow("address")) ?: ""
                        val body    = cursor.getString(cursor.getColumnIndexOrThrow("body")) ?: ""
                        val date    = cursor.getLong(cursor.getColumnIndexOrThrow("date"))
                        val ts      = java.time.Instant.ofEpochMilli(date)
                            .atOffset(java.time.ZoneOffset.UTC)
                            .format(java.time.format.DateTimeFormatter.ISO_OFFSET_DATE_TIME)
                        Log.d(TAG, "New SMS #$id")
                        broadcastToAll { it.sendSmsReceived(address, body, lookupContact(address), ts) }
                        if (id > lastSmsId) lastSmsId = id
                    }
                }
            } catch (e: Exception) {
                Log.e(TAG, "Cannot read the SMS inbox: ${e.message}")
            }
        }.start()
    }

    // ------------------------------------------------------------------ //
    //  Helpers                                                              //
    // ------------------------------------------------------------------ //

    private fun broadcastToAll(action: (CommWebSocket) -> Unit) {
        webSockets.values.forEach { action(it) }
    }

    private fun loadServers(): List<ServerConfig> {
        val json = prefs.getString("servers", "[]") ?: "[]"
        val type = object : TypeToken<List<ServerConfig>>() {}.type
        return gson.fromJson(json, type) ?: emptyList()
    }

    private fun lookupContact(phoneNumber: String): String {
        if (phoneNumber.isBlank()) return ""
        val uri = ContactsContract.PhoneLookup.CONTENT_FILTER_URI
            .buildUpon()
            .appendPath(phoneNumber)
            .build()
        val cursor: Cursor? = contentResolver.query(
            uri,
            arrayOf(ContactsContract.PhoneLookup.DISPLAY_NAME),
            null, null, null
        )
        return cursor?.use {
            if (it.moveToFirst())
                it.getString(it.getColumnIndexOrThrow(ContactsContract.PhoneLookup.DISPLAY_NAME))
            else ""
        } ?: ""
    }

    // ------------------------------------------------------------------ //
    //  Foreground notification                                              //
    // ------------------------------------------------------------------ //

    private fun createNotificationChannel() {
        val channel = NotificationChannel(
            NOTIF_CHANNEL,
            getString(R.string.notif_channel),
            NotificationManager.IMPORTANCE_LOW
        ).apply {
            description = getString(R.string.notif_channel_description)
        }
        getSystemService(NotificationManager::class.java).createNotificationChannel(channel)
    }

    private fun updateNotification() {
        val connected = online.size
        val total     = webSockets.size
        val text      = when {
            total == 0         -> getString(R.string.notif_none)
            connected == 0     -> getString(R.string.notif_offline)
            connected == total -> resources.getQuantityString(R.plurals.notif_all, connected, connected)
            else               -> resources.getQuantityString(R.plurals.notif_some, total, connected, total)
        }
        getSystemService(NotificationManager::class.java).notify(NOTIF_ID, buildNotification(text))
    }

    private fun buildNotification(text: String): Notification {
        val pendingIntent = PendingIntent.getActivity(
            this, 0,
            Intent(this, MainActivity::class.java),
            PendingIntent.FLAG_IMMUTABLE
        )
        return NotificationCompat.Builder(this, NOTIF_CHANNEL)
            .setContentTitle(getString(R.string.app_name))
            .setContentText(text)
            .setSmallIcon(R.drawable.ic_notification)
            .setContentIntent(pendingIntent)
            .setOngoing(true)
            .setForegroundServiceBehavior(NotificationCompat.FOREGROUND_SERVICE_IMMEDIATE)
            .build()
    }
}
