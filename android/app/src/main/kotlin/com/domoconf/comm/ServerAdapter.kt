// SPDX-License-Identifier: AGPL-3.0-or-later
package com.domoconf.comm

import android.view.LayoutInflater
import android.view.View
import android.view.ViewGroup
import android.widget.ImageButton
import android.widget.TextView
import androidx.recyclerview.widget.RecyclerView

/**
 * Adapter for the list of configured servers.
 * Shows the connection state of each server as a dot (green when connected, grey otherwise).
 */
class ServerAdapter(
    private val servers: MutableList<ServerConfig>,
    private val onEdit: (ServerConfig) -> Unit,
    private val onDelete: (ServerConfig) -> Unit,
) : RecyclerView.Adapter<ServerAdapter.ViewHolder>() {

    private val onlineIds = mutableSetOf<String>()

    /** Called on the main thread when one server connects or disconnects. */
    fun setOnline(id: String, online: Boolean) {
        if (online) onlineIds.add(id) else onlineIds.remove(id)
        val idx = servers.indexOfFirst { it.id == id }
        if (idx >= 0) notifyItemChanged(idx)
    }

    /** Replaces the whole state, for instance when the screen comes back in front. */
    fun setOnlineIds(ids: Set<String>) {
        onlineIds.clear()
        onlineIds.addAll(ids)
        notifyDataSetChanged()
    }

    override fun onCreateViewHolder(parent: ViewGroup, viewType: Int): ViewHolder {
        val view = LayoutInflater.from(parent.context)
            .inflate(R.layout.item_server, parent, false)
        return ViewHolder(view)
    }

    override fun onBindViewHolder(holder: ViewHolder, position: Int) {
        val cfg = servers[position]
        holder.bind(cfg, onlineIds.contains(cfg.id))
        holder.btnEdit.setOnClickListener   { onEdit(cfg) }
        holder.btnDelete.setOnClickListener { onDelete(cfg) }
    }

    override fun getItemCount() = servers.size

    class ViewHolder(view: View) : RecyclerView.ViewHolder(view) {
        val tvName: TextView        = view.findViewById(R.id.tv_server_name)
        val tvUrl: TextView         = view.findViewById(R.id.tv_server_url)
        val dot: View               = view.findViewById(R.id.server_status_dot)
        val btnEdit: ImageButton    = view.findViewById(R.id.btn_edit)
        val btnDelete: ImageButton  = view.findViewById(R.id.btn_delete)

        fun bind(cfg: ServerConfig, isOnline: Boolean) {
            tvName.text = cfg.name
            tvUrl.text  = cfg.wsUrl
            dot.setBackgroundResource(
                if (isOnline) R.drawable.ic_status_online else R.drawable.ic_status_offline
            )
        }
    }
}
