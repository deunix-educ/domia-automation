// SPDX-License-Identifier: AGPL-3.0-or-later
package com.domoconf.comm

import java.util.UUID

/**
 * Configuration of one DomiNet server.
 * wsUrl is the full WebSocket URL, token included:
 *   wss://dominet.local/ws/android/device/xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx/
 */
data class ServerConfig(
    val id: String    = UUID.randomUUID().toString(),
    val name: String,
    val wsUrl: String,
)
