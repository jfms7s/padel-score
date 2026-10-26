package io.github.jfms7s.padelscore.settings

import okhttp3.HttpUrl.Companion.toHttpUrl
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import java.net.InetAddress

class ServerSettingsTest {
    private fun server(text: String) = parseServerUrl(text)?.toString()

    @Test
    fun https_urls_are_reduced_to_their_origin() {
        assertEquals("https://padel.example.com/", server("https://padel.example.com"))
        assertEquals("https://padel.example.com/", server("  https://padel.example.com/link?x=1#frag "))
        assertEquals("https://padel.example.com:8443/", server("https://padel.example.com:8443/api/"))
        assertEquals("https://padel.example.com/", server("https://user:pw@padel.example.com/"))
        assertEquals("https://padel.example.com/", server("HTTPS://Padel.Example.COM"))
    }

    @Test
    fun missing_scheme_means_https() {
        assertEquals("https://padel.example.com/", server("padel.example.com"))
        assertEquals("https://192.168.50.231/", server("192.168.50.231"))
    }

    @Test
    fun http_only_to_the_dev_hosts_the_manifest_allows() {
        assertEquals("http://10.0.2.2:8080/", server("http://10.0.2.2:8080"))
        assertEquals("http://localhost:8080/", server("http://localhost:8080/"))
        // Cleartext anywhere else would be refused by the network security config at runtime.
        assertNull(server("http://padel.example.com"))
        assertNull(server("http://192.168.50.231"))
    }

    @Test
    fun junk_is_refused() {
        assertNull(server(""))
        assertNull(server("   "))
        assertNull(server("ftp://padel.example.com"))
        assertNull(server("https://"))
        assertNull(server("padel example com"))
        assertNull(server("https://exa mple.com"))
    }

    @Test
    fun same_origin_compares_scheme_host_and_port() {
        val a = "https://padel.example.com/".toHttpUrl()
        assertTrue(sameOrigin(a, "https://padel.example.com:443/x".toHttpUrl()))
        assertFalse(sameOrigin(a, "https://other.example.com/".toHttpUrl()))
        assertFalse(sameOrigin(a, "https://padel.example.com:8443/".toHttpUrl()))
        assertFalse(sameOrigin("http://localhost/".toHttpUrl(), "https://localhost/".toHttpUrl()))
    }

    @Test
    fun display_drops_scheme_and_trailing_slash() {
        assertEquals("padel.example.com", displayUrl("https://padel.example.com/".toHttpUrl()))
        assertEquals("10.0.2.2:8080", displayUrl("http://10.0.2.2:8080/".toHttpUrl()))
    }

    private fun addresses(text: String) = parseAddressList(text)?.map { it.hostAddress }

    @Test
    fun addresses_split_on_commas_and_spaces() {
        assertEquals(listOf("192.168.50.231", "192.168.50.201"), addresses("192.168.50.231,192.168.50.201"))
        assertEquals(listOf("192.168.50.231", "192.168.50.201"), addresses(" 192.168.50.231 , 192.168.50.201 "))
        assertEquals(listOf("192.168.50.231", "192.168.50.201"), addresses("192.168.50.231 192.168.50.201"))
        assertEquals(listOf("10.0.0.1"), addresses("10.0.0.1, 10.0.0.1"))
        assertEquals(1, parseAddressList("fd00::1")?.size)
    }

    @Test
    fun empty_addresses_mean_none() {
        assertEquals(emptyList<String>(), addresses(""))
        assertEquals(emptyList<String>(), addresses(" , "))
    }

    @Test
    fun one_bad_address_rejects_them_all() {
        assertNull(addresses("192.168.50.231, 192.168.50.300"))
        assertNull(addresses("192.168.50.231, padel.example.com"))
        assertNull(addresses("192.168.50.231, 192.168.0"))
        assertNull(addresses("192.168.50.231; 192.168.50.201"))
        // Would reach Android's getByName as a host name (a DNS lookup) or as octal.
        assertNull(addresses("1:2"))
        assertNull(addresses("1.2.3.4:5"))
        assertNull(addresses("010.0.0.1"))
        assertNull(addresses("192.168.50.231, 1:2"))
    }

    @Test
    fun address_text_round_trips() {
        val list = listOf(InetAddress.getByName("192.168.50.231"), InetAddress.getByName("10.0.0.1"))
        assertEquals("192.168.50.231, 10.0.0.1", addressText(list))
        assertEquals(list, parseAddressList(addressText(list)))
        assertEquals("", addressText(emptyList()))
    }
}
