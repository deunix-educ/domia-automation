// SPDX-License-Identifier: AGPL-3.0-or-later
package com.domoconf.comm

import android.Manifest
import android.content.Intent
import android.content.pm.PackageManager
import android.os.Build
import android.os.Bundle
import android.view.LayoutInflater
import android.widget.EditText
import android.widget.Toast
import androidx.activity.result.contract.ActivityResultContracts
import androidx.appcompat.app.AlertDialog
import androidx.appcompat.app.AppCompatActivity
import androidx.core.content.ContextCompat
import androidx.recyclerview.widget.LinearLayoutManager
import com.google.gson.Gson
import com.google.gson.reflect.TypeToken
import com.domoconf.comm.databinding.ActivityMainBinding
import com.domoconf.comm.service.CommService

class MainActivity : AppCompatActivity() {

    private lateinit var binding: ActivityMainBinding
    private lateinit var adapter: ServerAdapter

    private val servers = mutableListOf<ServerConfig>()
    private val gson    = Gson()

    private val prefs by lazy {
        getSharedPreferences("domocomm_prefs", MODE_PRIVATE)
    }

    private val requiredPermissions = buildList {
        add(Manifest.permission.RECEIVE_SMS)
        add(Manifest.permission.READ_SMS)
        add(Manifest.permission.READ_PHONE_STATE)
        add(Manifest.permission.READ_CALL_LOG)
        add(Manifest.permission.SEND_SMS)
        add(Manifest.permission.READ_CONTACTS)
        @Suppress("DEPRECATION")
        add(Manifest.permission.PROCESS_OUTGOING_CALLS)
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            add(Manifest.permission.POST_NOTIFICATIONS)
        }
    }

    private val permissionLauncher = registerForActivityResult(
        ActivityResultContracts.RequestMultiplePermissions()
    ) { _ ->
        startWithGrantedPermissions()
    }

    private fun isGranted(permission: String) =
        ContextCompat.checkSelfPermission(this, permission) == PackageManager.PERMISSION_GRANTED

    /**
     * Starts the service with the permissions that were granted. The relay needs the SMS permissions to be of any
     * use; the others (calls, contacts, notifications) only limit what it can do, and the missing ones are listed.
     */
    private fun startWithGrantedPermissions() {
        val canRelaySms = isGranted(Manifest.permission.SEND_SMS) || isGranted(Manifest.permission.READ_SMS)
        if (!canRelaySms) {
            Toast.makeText(this, getString(R.string.permissions_required), Toast.LENGTH_LONG).show()
            return
        }
        val missing = buildList {
            if (!isGranted(Manifest.permission.SEND_SMS) || !isGranted(Manifest.permission.READ_SMS) ||
                !isGranted(Manifest.permission.RECEIVE_SMS)) add(getString(R.string.perm_sms))
            if (!isGranted(Manifest.permission.READ_PHONE_STATE)) add(getString(R.string.perm_phone))
            if (!isGranted(Manifest.permission.READ_CALL_LOG)) add(getString(R.string.perm_call_log))
            if (!isGranted(Manifest.permission.READ_CONTACTS)) add(getString(R.string.perm_contacts))
        }
        if (missing.isNotEmpty()) {
            Toast.makeText(this, getString(R.string.permissions_partial, missing.joinToString(", ")), Toast.LENGTH_LONG).show()
        }
        startService()
    }

    // ------------------------------------------------------------------ //
    //  Life cycle                                                           //
    // ------------------------------------------------------------------ //

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        binding = ActivityMainBinding.inflate(layoutInflater)
        setContentView(binding.root)

        migrateOldPrefs()

        servers.addAll(loadServers())

        adapter = ServerAdapter(
            servers,
            onEdit   = { cfg -> showDialog(cfg) },
            onDelete = { cfg -> confirmDelete(cfg) },
        )

        binding.recyclerServers.layoutManager = LinearLayoutManager(this)
        binding.recyclerServers.adapter       = adapter

        binding.fabAdd.setOnClickListener      { showDialog(null) }
        binding.btnStartStop.setOnClickListener { toggleService() }

        refreshEmptyState()
    }

    override fun onStart() {
        super.onStart()
        // The service tells the screen when a server connects or disconnects: that turns its dot green or grey.
        CommService.statusListener = { id, online -> adapter.setOnline(id, online) }
        adapter.setOnlineIds(CommService.onlineIds())
    }

    override fun onStop() {
        CommService.statusListener = null
        super.onStop()
    }

    override fun onResume() {
        super.onResume()
        updateServiceButton()
    }

    // ------------------------------------------------------------------ //
    //  Servers                                                              //
    // ------------------------------------------------------------------ //

    private fun showDialog(existing: ServerConfig?) {
        val dialogView = LayoutInflater.from(this).inflate(R.layout.dialog_server, null)
        val editName   = dialogView.findViewById<EditText>(R.id.edit_name)
        val editUrl    = dialogView.findViewById<EditText>(R.id.edit_url)

        if (existing != null) {
            editName.setText(existing.name)
            editUrl.setText(existing.wsUrl)
        }

        val title = if (existing == null) R.string.dialog_add_server else R.string.dialog_edit_server

        AlertDialog.Builder(this)
            .setTitle(title)
            .setView(dialogView)
            .setPositiveButton(R.string.save) { _, _ ->
                val name = editName.text.toString().trim()
                val url  = editUrl.text.toString().trim()

                if (name.isEmpty() || url.isEmpty()) {
                    Toast.makeText(this, R.string.fill_all_fields, Toast.LENGTH_SHORT).show()
                    return@setPositiveButton
                }

                if (!url.startsWith("ws://") && !url.startsWith("wss://")) {
                    Toast.makeText(this, R.string.url_invalid, Toast.LENGTH_SHORT).show()
                    return@setPositiveButton
                }

                if (existing == null) {
                    servers.add(ServerConfig(name = name, wsUrl = url))
                    adapter.notifyItemInserted(servers.size - 1)
                } else {
                    val idx = servers.indexOfFirst { it.id == existing.id }
                    if (idx >= 0) {
                        servers[idx] = existing.copy(name = name, wsUrl = url)
                        adapter.notifyItemChanged(idx)
                    }
                }

                saveServers()
                refreshEmptyState()
                restartServiceIfRunning()
                Toast.makeText(this, R.string.saved, Toast.LENGTH_SHORT).show()
            }
            .setNegativeButton(R.string.cancel, null)
            .show()
    }

    private fun confirmDelete(cfg: ServerConfig) {
        AlertDialog.Builder(this)
            .setTitle(R.string.confirm_delete)
            .setMessage(cfg.name)
            .setPositiveButton(R.string.delete) { _, _ ->
                val idx = servers.indexOfFirst { it.id == cfg.id }
                if (idx >= 0) {
                    servers.removeAt(idx)
                    adapter.notifyItemRemoved(idx)
                }
                saveServers()
                refreshEmptyState()
                restartServiceIfRunning()
            }
            .setNegativeButton(R.string.cancel, null)
            .show()
    }

    // ------------------------------------------------------------------ //
    //  Service                                                              //
    // ------------------------------------------------------------------ //

    private fun toggleService() {
        if (CommService.isRunning) {
            stopService(Intent(this, CommService::class.java))
            // onDestroy() runs later: the screen is updated at once
            binding.btnStartStop.text = getString(R.string.start_service)
            binding.statusIndicator.setBackgroundResource(R.drawable.ic_status_offline)
            binding.tvStatus.text = getString(R.string.status_stopped)
        } else {
            if (servers.isEmpty()) {
                Toast.makeText(this, R.string.no_servers, Toast.LENGTH_SHORT).show()
                return
            }
            requestPermissionsAndStart()
        }
    }

    private fun requestPermissionsAndStart() {
        val missing = requiredPermissions.filter { !isGranted(it) }
        if (missing.isEmpty()) {
            startService()
        } else {
            permissionLauncher.launch(missing.toTypedArray())
        }
    }

    private fun startService() {
        ContextCompat.startForegroundService(this, Intent(this, CommService::class.java))
        binding.btnStartStop.text = getString(R.string.stop_service)
        binding.statusIndicator.setBackgroundResource(R.drawable.ic_status_online)
        binding.tvStatus.text = getString(R.string.status_connecting)
        Toast.makeText(this, R.string.service_started, Toast.LENGTH_SHORT).show()
    }

    private fun restartServiceIfRunning() {
        if (CommService.isRunning) {
            // Reloads the connections in place, without stop/start (avoids a race)
            val intent = Intent(this, CommService::class.java).apply {
                action = CommService.ACTION_RELOAD_SERVERS
            }
            ContextCompat.startForegroundService(this, intent)
        }
    }

    private fun updateServiceButton() {
        if (CommService.isRunning) {
            binding.btnStartStop.text = getString(R.string.stop_service)
            binding.statusIndicator.setBackgroundResource(R.drawable.ic_status_online)
            binding.tvStatus.text = getString(R.string.status_running)
        } else {
            binding.btnStartStop.text = getString(R.string.start_service)
            binding.statusIndicator.setBackgroundResource(R.drawable.ic_status_offline)
            binding.tvStatus.text = getString(R.string.status_stopped)
        }
    }

    // ------------------------------------------------------------------ //
    //  Storage                                                              //
    // ------------------------------------------------------------------ //

    private fun loadServers(): List<ServerConfig> {
        val json = prefs.getString("servers", "[]") ?: "[]"
        val type = object : TypeToken<List<ServerConfig>>() {}.type
        return gson.fromJson(json, type) ?: emptyList()
    }

    private fun saveServers() {
        prefs.edit().putString("servers", gson.toJson(servers)).apply()
    }

    /** Migration from the old single-server settings (server_url + token). */
    private fun migrateOldPrefs() {
        if (prefs.contains("servers")) return
        val oldUrl   = prefs.getString("server_url", "") ?: ""
        val oldToken = prefs.getString("token", "")       ?: ""
        if (oldUrl.isEmpty() || oldToken.isEmpty()) return

        val wsUrl = oldUrl.trimEnd('/')
            .replace("^http://".toRegex(),  "ws://")
            .replace("^https://".toRegex(), "wss://")
        val config = ServerConfig(name = getString(R.string.default_server_name), wsUrl = "$wsUrl/ws/android/device/$oldToken/")

        prefs.edit()
            .putString("servers", gson.toJson(listOf(config)))
            .remove("server_url")
            .remove("token")
            .apply()
    }

    private fun refreshEmptyState() {
        binding.tvEmpty.visibility =
            if (servers.isEmpty()) android.view.View.VISIBLE else android.view.View.GONE
    }
}
