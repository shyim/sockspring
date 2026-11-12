package de.shyim.sockspring

import android.content.Context
import java.net.InetAddress
import org.json.JSONObject

data class AppConfig(
    val listen: String = "::",
    val port: String = "1080",
    val workers: String = "4",
    val username: String = "",
    val password: String = "",
    val auth: Boolean = false,
    val udpListen: String = "",
    val udpPort: String = "0",
    val bindV4: String = "0.0.0.0",
    val bindV6: String = "::",
    val bindInterface: String = "",
    val ipv6Only: Boolean = false,
    val noDelay: Boolean = true,
    val autoStart: Boolean = false,
) {
    fun errors(): Map<String, String> = buildMap {
        fun ip(key: String, value: String, optional: Boolean = false) {
            if ((!optional || value.isNotBlank()) && !numericAddress(value))
                put(key, "Enter a valid IP address")
        }
        ip("listen", listen)
        if (port.toIntOrNull() !in 1..65535) put("port", "Use a port from 1 to 65535")
        if (workers.toIntOrNull() !in 1..32) put("workers", "Choose between 1 and 32 workers")
        if (auth) {
            if (username.isEmpty() || username.toByteArray().size > 255)
                put("username", "Enter a username (up to 255 bytes)")
            if (password.isEmpty() || password.toByteArray().size > 255)
                put("password", "Enter a password (up to 255 bytes)")
        }
        ip("udpListen", udpListen, true)
        val ports = udpPort.split('-')
        val first = ports.firstOrNull()?.toIntOrNull()
        val last = ports.lastOrNull()?.toIntOrNull()
        if (
            ports.size !in 1..2 ||
                first == null ||
                last == null ||
                first !in 0..65535 ||
                last !in 0..65535 ||
                first > last ||
                (first == 0 && last != 0)
        )
            put("udpPort", "Use 0, a port, or a range such as 4000-4100")
        ip("bindV4", bindV4, true)
        ip("bindV6", bindV6, true)
        if (bindV4.isNotBlank() && ':' in bindV4) put("bindV4", "Enter an IPv4 address")
        if (bindV6.isNotBlank() && ':' !in bindV6) put("bindV6", "Enter an IPv6 address")
        if (bindInterface.length > 15 || '\u0000' in bindInterface)
            put("interface", "Use a valid interface name, up to 15 characters")
    }

    fun nativeJson(): String =
        JSONObject()
            .apply {
                put("listen", listen)
                put("port", port.toInt())
                put("workers", workers.toInt())
                put("username", if (auth) username else "")
                put("password", if (auth) password else "")
                put("udp_listen", udpListen)
                put("udp_port", udpPort)
                put("bind_ipv4", bindV4)
                put("bind_ipv6", bindV6)
                put("interface", bindInterface)
                put("ipv6_only", ipv6Only)
                put("tcp_nodelay", noDelay)
            }
            .toString()

    fun save(context: Context) {
        context
            .getSharedPreferences("SocksPrefs", Context.MODE_PRIVATE)
            .edit()
            .apply {
                putString("ListenAddr", listen)
                putInt("ListenPort", port.toInt())
                putInt("Workers", workers.toInt())
                putString("AuthUser", username)
                putString("AuthPass", password)
                putBoolean("AuthEnabled", auth)
                putString("UDPListenAddr", udpListen)
                putString("UDPListenPort", udpPort)
                putString("BindIPv4Addr", bindV4)
                putString("BindIPv6Addr", bindV6)
                putString("BindIface", bindInterface)
                putBoolean("ListenIPv6Only", ipv6Only)
                putBoolean("TcpNoDelay", noDelay)
                putBoolean("AutoStart", autoStart)
            }
            .apply()
    }

    companion object {
        // Android's Patterns.IP_ADDRESS accepts IPv4 only. Validate IPv6 too,
        // without allowing hostnames or doing DNS on the UI thread.
        internal fun numericAddress(value: String): Boolean {
            if (':' in value) {
                if (!value.matches(Regex("[0-9a-fA-F:.]+"))) return false
                return runCatching { InetAddress.getByName(value) }.isSuccess
            }
            val parts = value.split('.')
            return parts.size == 4 &&
                parts.all { part ->
                    part.isNotEmpty() &&
                        part.all { it in '0'..'9' } &&
                        (part.length == 1 || !part.startsWith('0')) &&
                        part.toIntOrNull() in 0..255
                }
        }

        fun load(context: Context): AppConfig {
            val p = context.getSharedPreferences("SocksPrefs", Context.MODE_PRIVATE)
            return AppConfig(
                listen = p.getString("ListenAddr", "::")!!,
                port = p.getInt("ListenPort", 1080).toString(),
                workers = p.getInt("Workers", 4).toString(),
                username = p.getString("AuthUser", "")!!,
                password = p.getString("AuthPass", "")!!,
                auth =
                    p.getBoolean(
                        "AuthEnabled",
                        !p.getString("AuthUser", "").isNullOrEmpty() &&
                            !p.getString("AuthPass", "").isNullOrEmpty(),
                    ),
                udpListen = p.getString("UDPListenAddr", "")!!,
                udpPort = p.getString("UDPListenPort", "0")!!,
                bindV4 = p.getString("BindIPv4Addr", "0.0.0.0")!!,
                bindV6 = p.getString("BindIPv6Addr", "::")!!,
                bindInterface = p.getString("BindIface", "")!!,
                ipv6Only = p.getBoolean("ListenIPv6Only", false),
                noDelay = p.getBoolean("TcpNoDelay", true),
                autoStart = p.getBoolean("AutoStart", false),
            )
        }
    }
}
