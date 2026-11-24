package com.roadmesh.android

import android.content.Context
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import android.security.keystore.StrongBoxUnavailableException
import com.roadmesh.core.HardwareKeys
import java.security.KeyPairGenerator
import java.security.KeyStore
import java.security.PrivateKey
import java.security.Signature
import java.security.interfaces.ECPublicKey
import java.security.spec.ECGenParameterSpec

/**
 * Pseudonym and device keys in the Android Keystore (AUDIT S-02): generated in secure
 * hardware (StrongBox where available, otherwise the TEE), never exportable. The Rust core
 * asks for signatures through [HardwareKeys]; it never sees a private key.
 */
class KeystoreKeys(context: Context) : HardwareKeys {
    private val ks: KeyStore = KeyStore.getInstance(PROVIDER).apply { load(null) }
    private val prefs = context.getSharedPreferences(PREFS, Context.MODE_PRIVATE)

    /** Generate (or reuse) a P-256 signing key under `alias`; returns its compressed public key. */
    fun generate(alias: String): ByteArray {
        if (!ks.containsAlias(alias)) {
            try {
                create(alias, strongBox = true)
            } catch (e: StrongBoxUnavailableException) {
                create(alias, strongBox = false)
            }
        }
        val cert = ks.getCertificate(alias) ?: error("no key")
        return Crypto.compress(cert.publicKey as ECPublicKey)
    }

    private fun create(alias: String, strongBox: Boolean) {
        val spec = KeyGenParameterSpec.Builder(alias, KeyProperties.PURPOSE_SIGN)
            .setAlgorithmParameterSpec(ECGenParameterSpec("secp256r1"))
            .setDigests(KeyProperties.DIGEST_SHA256)
            .setIsStrongBoxBacked(strongBox)
            .build()
        KeyPairGenerator.getInstance(KeyProperties.KEY_ALGORITHM_EC, PROVIDER).run {
            initialize(spec)
            generateKeyPair()
        }
    }

    /** Raw r‖s signature over `message` with the key under `alias`, or null. */
    fun signWith(alias: String, message: ByteArray): ByteArray? {
        val key = ks.getKey(alias, null) as? PrivateKey ?: return null
        val der = Signature.getInstance("SHA256withECDSA").run {
            initSign(key)
            update(message)
            sign()
        }
        return Crypto.derToRaw(der)
    }

    /** Remember which key belongs to which certificate (digest hex → alias). */
    fun bind(certDigest: ByteArray, alias: String) {
        prefs.edit().putString(PREFIX + Crypto.hex(certDigest), alias).apply()
    }

    /** Delete keys that no longer have a certificate (expired or never issued). */
    fun deleteUnbound(keepAliases: Set<String>) {
        for (alias in ks.aliases().toList()) {
            if (alias.startsWith(PSEUDONYM_PREFIX) && alias !in keepAliases) ks.deleteEntry(alias)
        }
    }

    /** Aliases currently bound to certificates. */
    fun boundAliases(): Set<String> =
        prefs.all.filterKeys { it.startsWith(PREFIX) }.values.filterIsInstance<String>().toSet()

    override fun sign(certDigest: ByteArray, message: ByteArray): ByteArray {
        val alias = prefs.getString(PREFIX + Crypto.hex(certDigest), null) ?: return ByteArray(0)
        return signWith(alias, message) ?: ByteArray(0)
    }

    companion object {
        private const val PROVIDER = "AndroidKeyStore"
        private const val PREFS = "roadmesh_keys"
        private const val PREFIX = "cert_"
        const val DEVICE_ALIAS = "roadmesh_device"
        const val PSEUDONYM_PREFIX = "roadmesh_pseudo_"
    }
}
