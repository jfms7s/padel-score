package io.github.jfms7s.padelscore.net

import okhttp3.Dns
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import java.net.InetAddress
import java.net.UnknownHostException
import java.util.concurrent.CountDownLatch
import java.util.concurrent.Executors

class LanFallbackDnsTest {
    private val host = "padel.example.test"
    private val lan = LanFallbackDns.parseAddresses("192.168.50.231, 192.168.50.201")
    private val publicIp = InetAddress.getByName("203.0.113.7")
    private val privateIp = InetAddress.getByName("192.168.50.11")

    private class FakeDns(private val answer: (String) -> List<InetAddress>) : Dns {
        val asked = mutableListOf<String>()
        override fun lookup(hostname: String): List<InetAddress> {
            synchronized(asked) { asked += hostname }
            return answer(hostname)
        }
    }

    private fun dns(system: Dns, timeoutMillis: Long = 1_000) = LanFallbackDns(host, lan, system, timeoutMillis)

    @Test
    fun private_answer_is_used_as_is() {
        val result = dns(FakeDns { listOf(privateIp) }).lookup(host)
        assertEquals(listOf(privateIp), result)
    }

    @Test
    fun public_only_answer_puts_the_lan_addresses_first() {
        val result = dns(FakeDns { listOf(publicIp) }).lookup(host)
        assertEquals(lan + publicIp, result)
    }

    @Test
    fun failed_lookup_uses_the_lan_addresses() {
        val result = dns(FakeDns { throw UnknownHostException(it) }).lookup(host)
        assertEquals(lan, result)
    }

    @Test
    fun hung_lookup_gives_up_and_uses_the_lan_addresses() {
        val release = CountDownLatch(1)
        val started = System.nanoTime()
        try {
            val result = dns(FakeDns { release.await(); listOf(publicIp) }, timeoutMillis = 200).lookup(host)
            val elapsedMs = (System.nanoTime() - started) / 1_000_000
            assertEquals(lan, result)
            assertTrue("took $elapsedMs ms", elapsedMs < 2_000)
        } finally {
            release.countDown()
        }
    }

    @Test
    fun host_match_ignores_case() {
        val result = dns(FakeDns { listOf(publicIp) }).lookup("PADEL.Example.TEST")
        assertEquals(lan + publicIp, result)
    }

    @Test
    fun other_hosts_go_straight_to_the_system() {
        val system = FakeDns { throw UnknownHostException(it) }
        val error = runCatching { dns(system).lookup("other.example.test") }.exceptionOrNull()
        assertTrue(error is UnknownHostException)
        assertEquals(listOf("other.example.test"), system.asked)
    }

    @Test
    fun without_fallback_the_system_answer_and_errors_pass_through() {
        val failing = LanFallbackDns(host, emptyList(), FakeDns { throw UnknownHostException(it) })
        assertTrue(runCatching { failing.lookup(host) }.exceptionOrNull() is UnknownHostException)
        val public = LanFallbackDns(host, emptyList(), FakeDns { listOf(publicIp) })
        assertEquals(listOf(publicIp), public.lookup(host))
    }

    @Test
    fun parse_takes_only_ip_literals() {
        val parsed = LanFallbackDns.parseAddresses(" 192.168.50.231,,not-a-host.example, 999.1.1.1, fd00::1 ,10.0.0.5")
        assertEquals(
            listOf("192.168.50.231", "fd00::1", "10.0.0.5").map(InetAddress::getByName),
            parsed,
        )
        assertEquals(emptyList<InetAddress>(), LanFallbackDns.parseAddresses(""))
    }

    @Test
    fun private_ranges() {
        listOf("10.1.2.3", "172.16.0.1", "192.168.50.231", "169.254.1.1", "127.0.0.1", "fd00::1", "fe80::1")
            .forEach { assertTrue(it, InetAddress.getByName(it).isPrivate()) }
        listOf("203.0.113.7", "8.8.8.8", "2001:db8::1", "172.32.0.1")
            .forEach { assertTrue(it, !InetAddress.getByName(it).isPrivate()) }
    }

    @Test
    fun concurrent_lookups_each_get_an_answer() {
        val pool = Executors.newFixedThreadPool(4)
        val d = dns(FakeDns { listOf(publicIp) })
        val results = (1..8).map { pool.submit<List<InetAddress>> { d.lookup(host) } }.map { it.get() }
        pool.shutdown()
        results.forEach { assertEquals(lan + publicIp, it) }
    }
}
