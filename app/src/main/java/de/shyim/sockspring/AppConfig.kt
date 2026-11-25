package de.shyim.sockspring

import android.content.Context
import java.net.InetAddress
import java.security.SecureRandom
import org.json.JSONArray
import org.json.JSONObject

/** A device allowed to connect to the WireGuard server. [host] is its last address octet. */
data class WgPeer(
    val name: String,
    val privateKey: String,
    val publicKey: String,
    val presharedKey: String,
    val host: Int,
)

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
    val socks: Boolean = true,
    val wg: Boolean = false,
    val wgPort: String = "51820",
    val wgPrivateKey: String = "",
    val wgAddress: String = "10.77.0.1",
    val wgEndpoint: String = "",
    val wgDns: String = "",
    val wgMtu: String = "1420",
    val wgPeers: List<WgPeer> = emptyList(),
) {
    /** The /24 the tunnel lives in, without its last octet: "10.77.0". */
    private val wgNetwork
        get() = wgAddress.substringBeforeLast('.')

    fun peerAddress4(peer: WgPeer) = "$wgNetwork.${peer.host}"

    fun peerAddress6(peer: WgPeer) = "$WG_PREFIX6${peer.host.toString(16)}"

    /** The lowest free host number, or null when no more devices fit. */
    fun nextPeerHost(): Int? =
        if (wgPeers.size >= MAX_WG_PEERS) null
        else (2..254).firstOrNull { host -> wgPeers.none { it.host == host } }

    /** A configuration file for the official WireGuard apps and wg-quick. */
    fun clientConfig(peer: WgPeer, serverPublicKey: String, host: String): String {
        val endpoint = if (':' in host) "[$host]:$wgPort" else "$host:$wgPort"
        val dns = wgDns.split(',').map { it.trim() }.filter { it.isNotEmpty() }
        return """
            [Interface]
            PrivateKey = ${peer.privateKey}
            Address = ${peerAddress4(peer)}/32, ${peerAddress6(peer)}/128
            DNS = ${if (dns.isEmpty()) wgAddress else dns.joinToString(", ")}
            MTU = $wgMtu

            [Peer]
            PublicKey = $serverPublicKey
            PresharedKey = ${peer.presharedKey}
            AllowedIPs = 0.0.0.0/0, ::/0
            Endpoint = $endpoint
            PersistentKeepalive = 25
            """
            .trimIndent() + "\n"
    }

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
        if (!socks && !wg) put("socks", "Turn on SOCKS5, WireGuard, or both")
        if (wg) {
            if (wgPort.toIntOrNull() !in 1..65535) put("wgPort", "Use a port from 1 to 65535")
            if (':' in wgAddress || !numericAddress(wgAddress) || !wgAddress.endsWith(".1"))
                put("wgAddress", "Use an IPv4 address ending in .1, such as 10.77.0.1")
            if (wgMtu.toIntOrNull() !in 1280..1500) put("wgMtu", "Use an MTU from 1280 to 1500")
            if (wgEndpoint.length > 253 || !wgEndpoint.matches(Regex("[A-Za-z0-9.:_-]*")))
                put("wgEndpoint", "Enter a hostname or IP address without a port")
            if (
                wgDns.isNotBlank() &&
                    !wgDns.split(',').all { numericAddress(it.trim()) }
            )
                put("wgDns", "Enter IP addresses separated by commas")
        }
    }

    /** [resolvers] answer DNS queries that clients send to the tunnel address. */
    fun nativeJson(resolvers: List<String> = emptyList()): String =
        JSONObject()
            .apply {
                put("socks", socks)
                put(
                    "wireguard",
                    JSONObject().apply {
                        put("enabled", wg)
                        put("listen", listen)
                        put("port", wgPort.toIntOrNull() ?: 51820)
                        put("private_key", wgPrivateKey)
                        put("address4", wgAddress)
                        put("address6", "${WG_PREFIX6}1")
                        put("mtu", wgMtu.toIntOrNull() ?: 1420)
                        put("dns", JSONArray(resolvers))
                        put(
                            "peers",
                            JSONArray(
                                wgPeers.map { peer ->
                                    JSONObject().apply {
                                        put("public_key", peer.publicKey)
                                        put("preshared_key", peer.presharedKey)
                                        put(
                                            "allowed_ips",
                                            JSONArray(
                                                listOf(
                                                    "${peerAddress4(peer)}/32",
                                                    "${peerAddress6(peer)}/128",
                                                )
                                            ),
                                        )
                                    }
                                }
                            ),
                        )
                    },
                )
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
                putBoolean("SocksEnabled", socks)
                putBoolean("WgEnabled", wg)
                putInt("WgPort", wgPort.toIntOrNull() ?: 51820)
                putString("WgPrivateKey", wgPrivateKey)
                putString("WgAddress", wgAddress)
                putString("WgEndpoint", wgEndpoint)
                putString("WgDns", wgDns)
                putInt("WgMtu", wgMtu.toIntOrNull() ?: 1420)
                putString("WgPeers", peersToJson(wgPeers))
            }
            .apply()
    }

    companion object {
        const val WG_PREFIX6 = "fd77:736f:636b::"
        const val MAX_WG_PEERS = 64

        /** A fresh Curve25519 private key or preshared key, base64 encoded. */
        fun randomKey(): String {
            val bytes = ByteArray(32).also { SecureRandom().nextBytes(it) }
            // Clamp like `wg genkey` so the stored key equals the one actually used.
            bytes[0] = (bytes[0].toInt() and 248).toByte()
            bytes[31] = ((bytes[31].toInt() and 127) or 64).toByte()
            return base64(bytes)
        }

        // java.util.Base64 needs API 26 and android.util.Base64 is absent in unit tests.
        internal fun base64(bytes: ByteArray): String {
            val alphabet = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/"
            return bytes
                .toList()
                .chunked(3)
                .joinToString("") { chunk ->
                    val bits = chunk.fold(0) { acc, b -> (acc shl 8) or (b.toInt() and 255) } shl
                        (8 * (3 - chunk.size))
                    (0..3).joinToString("") { index ->
                        if (index <= chunk.size) alphabet[(bits shr (18 - 6 * index)) and 63].toString()
                        else "="
                    }
                }
        }

        internal fun peersToJson(peers: List<WgPeer>): String =
            JSONArray(
                    peers.map {
                        JSONObject()
                            .put("name", it.name)
                            .put("private_key", it.privateKey)
                            .put("public_key", it.publicKey)
                            .put("preshared_key", it.presharedKey)
                            .put("host", it.host)
                    }
                )
                .toString()

        internal fun peersFromJson(json: String): List<WgPeer> =
            runCatching {
                    val array = JSONArray(json)
                    (0 until array.length()).map { index ->
                        val o = array.getJSONObject(index)
                        WgPeer(
                            name = o.getString("name"),
                            privateKey = o.getString("private_key"),
                            publicKey = o.getString("public_key"),
                            presharedKey = o.getString("preshared_key"),
                            host = o.getInt("host"),
                        )
                    }
                }
                .getOrDefault(emptyList())

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
            // The server key is created once and kept, so device configurations stay valid.
            val privateKey =
                p.getString("WgPrivateKey", "").orEmpty().ifEmpty {
                    randomKey().also { p.edit().putString("WgPrivateKey", it).apply() }
                }
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
                socks = p.getBoolean("SocksEnabled", true),
                wg = p.getBoolean("WgEnabled", false),
                wgPort = p.getInt("WgPort", 51820).toString(),
                wgPrivateKey = privateKey,
                wgAddress = p.getString("WgAddress", "10.77.0.1")!!,
                wgEndpoint = p.getString("WgEndpoint", "")!!,
                wgDns = p.getString("WgDns", "")!!,
                wgMtu = p.getInt("WgMtu", 1420).toString(),
                wgPeers = peersFromJson(p.getString("WgPeers", "[]")!!),
            )
        }
    }
}
