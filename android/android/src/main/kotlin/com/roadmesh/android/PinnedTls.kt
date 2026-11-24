package com.roadmesh.android

import java.io.ByteArrayInputStream
import java.net.InetSocketAddress
import java.net.Socket
import java.security.KeyStore
import java.security.MessageDigest
import java.security.Principal
import java.security.PrivateKey
import java.security.cert.CertificateException
import java.security.cert.CertificateFactory
import java.security.cert.X509Certificate
import javax.net.ssl.SNIHostName
import javax.net.ssl.SSLContext
import javax.net.ssl.SSLEngine
import javax.net.ssl.SSLSocket
import javax.net.ssl.TrustManagerFactory
import javax.net.ssl.X509ExtendedKeyManager
import javax.net.ssl.X509TrustManager

/**
 * TLS 1.3 to the edge server (AUDIT S-16, S-17): the system's certificate validation
 * and host-name check, then the SPKI pin; the client certificate is the hardware
 * device key with the device certificate the server issued at enrolment.
 */
object PinnedTls {
    /** SHA-256 of a SubjectPublicKeyInfo (as `PublicKey.encoded` returns it). */
    fun spkiSha256(spki: ByteArray): ByteArray = MessageDigest.getInstance("SHA-256").digest(spki)

    /** True if `spki` hashes to one of `pins`. */
    fun pinMatches(spki: ByteArray, pins: List<ByteArray>): Boolean {
        val h = spkiSha256(spki)
        return pins.any { MessageDigest.isEqual(it, h) }
    }

    /** System validation, then the pin on the server's own certificate. */
    class PinningTrustManager(private val pins: List<ByteArray>, private val system: X509TrustManager) :
        X509TrustManager {
        override fun checkServerTrusted(chain: Array<out X509Certificate>, authType: String) {
            system.checkServerTrusted(chain, authType)
            val leaf = chain.firstOrNull() ?: throw CertificateException("no certificate")
            if (!pinMatches(leaf.publicKey.encoded, pins)) throw CertificateException("certificate pin mismatch")
        }

        override fun checkClientTrusted(chain: Array<out X509Certificate>, authType: String) =
            throw CertificateException("not a server")

        override fun getAcceptedIssuers(): Array<X509Certificate> = system.acceptedIssuers
    }

    /** Presents the device certificate with the device key that never leaves the Keystore. */
    class DeviceKeyManager(private val alias: String, certDer: ByteArray) : X509ExtendedKeyManager() {
        private val chain = arrayOf(
            CertificateFactory.getInstance("X.509").generateCertificate(ByteArrayInputStream(certDer)) as X509Certificate,
        )
        private val ks = KeyStore.getInstance("AndroidKeyStore").apply { load(null) }

        override fun chooseClientAlias(keyType: Array<out String>?, issuers: Array<out Principal>?, socket: Socket?) = alias
        override fun chooseEngineClientAlias(keyType: Array<out String>?, issuers: Array<out Principal>?, engine: SSLEngine?) = alias
        override fun getCertificateChain(a: String?): Array<X509Certificate>? = if (a == alias) chain else null
        override fun getPrivateKey(a: String?): PrivateKey? = if (a == alias) ks.getKey(alias, null) as? PrivateKey else null
        override fun getClientAliases(keyType: String?, issuers: Array<out Principal>?) = arrayOf(alias)
        override fun chooseServerAlias(keyType: String?, issuers: Array<out Principal>?, socket: Socket?): String? = null
        override fun getServerAliases(keyType: String?, issuers: Array<out Principal>?): Array<String>? = null
    }

    private fun systemTrust(): X509TrustManager {
        val f = TrustManagerFactory.getInstance(TrustManagerFactory.getDefaultAlgorithm())
        f.init(null as KeyStore?)
        return f.trustManagers.filterIsInstance<X509TrustManager>().first()
    }

    /** Connect and finish the handshake; throws on any failure (including a pin mismatch). */
    fun connect(host: String, port: Int, pins: List<ByteArray>, device: DeviceKeyManager?, timeoutMs: Int = 10_000): SSLSocket {
        val ctx = SSLContext.getInstance("TLSv1.3")
        ctx.init(device?.let { arrayOf(it) }, arrayOf(PinningTrustManager(pins, systemTrust())), null)
        val tcp = Socket()
        tcp.connect(InetSocketAddress(host, port), timeoutMs)
        val s = ctx.socketFactory.createSocket(tcp, host, port, true) as SSLSocket
        s.enabledProtocols = arrayOf("TLSv1.3")
        val p = s.sslParameters
        p.serverNames = listOf(SNIHostName(host))
        p.endpointIdentificationAlgorithm = "HTTPS" // host-name check
        if (android.os.Build.VERSION.SDK_INT >= 29) p.applicationProtocols = arrayOf("mqtt")
        s.sslParameters = p
        s.soTimeout = timeoutMs
        s.startHandshake()
        return s
    }
}
