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
}
