package de.shyim.sockspring

import org.junit.Assert.*
import org.junit.Test

class AppConfigTest {
    @Test
    fun defaultsIncludeValidIpv6Addresses() {
        assertTrue(AppConfig().errors().isEmpty())
    }

    @Test
    fun numericAddressesNeverAcceptHostnames() {
        for (ip in
            listOf(
                "::",
                "::1",
                "2001:db8::1",
                "::ffff:192.0.2.1",
                "127.0.0.1",
                "0.0.0.0",
            )) assertTrue(ip, AppConfig.numericAddress(ip))
        for (ip in
            listOf(
                "localhost",
                "example.com",
                "256.1.1.1",
                "1.2.3",
                "::g",
                "1::2::3",
                "",
                "01.2.3.4",
            )) assertFalse(ip, AppConfig.numericAddress(ip))
    }

    @Test
    fun invalidFieldsAreReportedBeforeSaving() {
        val errors =
            AppConfig(
                    port = "0",
                    workers = "abc",
                    auth = true,
                    username = "",
                    password = "",
                    udpPort = "4-2",
                    bindV4 = "::",
                    bindV6 = "127.0.0.1",
                )
                .errors()
        assertTrue(
            errors.keys.containsAll(
                listOf("port", "workers", "username", "password", "udpPort", "bindV4", "bindV6")
            )
        )
    }

    @Test
    fun credentialLimitCountsUtf8Bytes() {
        assertTrue(
            AppConfig(auth = true, username = "user", password = "é".repeat(128))
                .errors()
                .containsKey("password")
        )
        assertTrue(
            AppConfig(auth = true, username = "user", password = "é".repeat(127)).errors().isEmpty()
        )
    }

    @Test
    fun validUdpRangeAndOptionalAddresses() {
        assertTrue(
            AppConfig(udpPort = "4000-4100", udpListen = "", bindV4 = "", bindV6 = "")
                .errors()
                .isEmpty()
        )
        assertTrue(AppConfig(udpPort = "0-100").errors().containsKey("udpPort"))
    }

    private val peer =
        WgPeer(
            name = "Laptop",
            privateKey = "cHJpdmF0ZQ==",
            publicKey = "cHVibGlj",
            presharedKey = "c2hhcmVk",
            host = 2,
        )

    @Test
    fun wireGuardSettingsAreValidatedOnlyWhenEnabled() {
        val broken = AppConfig(wgPort = "0", wgAddress = "10.77.0.5", wgMtu = "900", wgDns = "dns.example")
        assertTrue(broken.errors().isEmpty())
        assertTrue(
            broken
                .copy(wg = true, wgEndpoint = "host:51820/x")
                .errors()
                .keys
                .containsAll(listOf("wgPort", "wgAddress", "wgMtu", "wgDns", "wgEndpoint"))
        )
        assertTrue(
            AppConfig(wg = true, wgEndpoint = "vpn.example.org", wgDns = "1.1.1.1, 2606:4700:4700::1111")
                .errors()
                .isEmpty()
        )
        assertTrue(AppConfig(socks = false).errors().containsKey("socks"))
        assertTrue(AppConfig(socks = false, wg = true).errors().isEmpty())
    }

    @Test
    fun devicesGetTheLowestFreeAddress() {
        val config = AppConfig(wgAddress = "192.168.99.1", wgPeers = listOf(peer, peer.copy(host = 4)))
        assertEquals(3, config.nextPeerHost())
        assertEquals("192.168.99.2", config.peerAddress4(peer))
        assertEquals("fd77:736f:636b::a", config.peerAddress6(peer.copy(host = 10)))
        val full = AppConfig(wgPeers = (2..65).map { peer.copy(host = it) })
        assertNull(full.nextPeerHost())
    }

    @Test
    fun clientConfigurationIsCompleteAndBracketsIpv6Endpoints() {
        val config = AppConfig(wg = true, wgPeers = listOf(peer))
        assertEquals(
            """
            [Interface]
            PrivateKey = cHJpdmF0ZQ==
            Address = 10.77.0.2/32, fd77:736f:636b::2/128
            DNS = 10.77.0.1
            MTU = 1420

            [Peer]
            PublicKey = c2VydmVy
            PresharedKey = c2hhcmVk
            AllowedIPs = 0.0.0.0/0, ::/0
            Endpoint = 192.168.43.1:51820
            PersistentKeepalive = 25

            """
                .trimIndent(),
            config.clientConfig(peer, "c2VydmVy", "192.168.43.1"),
        )
        val custom = config.copy(wgPort = "4000", wgDns = "9.9.9.9,1.1.1.1")
        val text = custom.clientConfig(peer, "c2VydmVy", "fd00::1")
        assertTrue(text.contains("Endpoint = [fd00::1]:4000\n"))
        assertTrue(text.contains("DNS = 9.9.9.9, 1.1.1.1\n"))
    }

    @Test
    fun engineConfigurationListsEveryDevice() {
        val json =
            org.json.JSONObject(
                AppConfig(wg = true, socks = false, wgPrivateKey = "a2V5", wgPeers = listOf(peer))
                    .nativeJson(listOf("192.0.2.53"))
            )
        assertFalse(json.getBoolean("socks"))
        val wireguard = json.getJSONObject("wireguard")
        assertTrue(wireguard.getBoolean("enabled"))
        assertEquals(51820, wireguard.getInt("port"))
        assertEquals("a2V5", wireguard.getString("private_key"))
        assertEquals("fd77:736f:636b::1", wireguard.getString("address6"))
        assertEquals("192.0.2.53", wireguard.getJSONArray("dns").getString(0))
        val device = wireguard.getJSONArray("peers").getJSONObject(0)
        assertEquals("cHVibGlj", device.getString("public_key"))
        assertEquals("10.77.0.2/32", device.getJSONArray("allowed_ips").getString(0))
        assertEquals("fd77:736f:636b::2/128", device.getJSONArray("allowed_ips").getString(1))
    }

    @Test
    fun devicesSurviveStorage() {
        val peers = listOf(peer, peer.copy(name = "Tablet \"2\"", host = 3))
        assertEquals(peers, AppConfig.peersFromJson(AppConfig.peersToJson(peers)))
        assertTrue(AppConfig.peersFromJson("not json").isEmpty())
    }

    @Test
    fun generatedKeysAreClampedBase64() {
        assertEquals("", AppConfig.base64(ByteArray(0)))
        assertEquals("Zg==", AppConfig.base64("f".toByteArray()))
        assertEquals("Zm8=", AppConfig.base64("fo".toByteArray()))
        assertEquals("Zm9vYmFy", AppConfig.base64("foobar".toByteArray()))
        assertEquals("/+8=", AppConfig.base64(byteArrayOf(-1, -17)))
        val key = java.util.Base64.getDecoder().decode(AppConfig.randomKey())
        assertEquals(32, key.size)
        assertEquals(0, key[0].toInt() and 7)
        assertEquals(64, key[31].toInt() and 192)
        assertNotEquals(AppConfig.randomKey(), AppConfig.randomKey())
    }
}
