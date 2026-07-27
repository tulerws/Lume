package com.tulerws.lume.mobile

import org.junit.Assert.assertEquals
import org.junit.Assert.assertThrows
import org.junit.Test

class LumeTransportCryptoTest {
    @Test
    fun decryptsTheRustWebCryptoFixture() {
        val key = LumeTransportCrypto.keyFromToken("lume-cross-platform-test")
        val envelope = LumeEncryptedEnvelope(
            nonce = "AAECAwQFBgcICQoL",
            ciphertext = "JXSIvh5kXBXm-lf4vWqiHmNTDrBOFzG-D8gPiTqOg_iyNwrvgk0EZBJi3hHNUbYTHYFUVT3R",
        )

        assertEquals(
            """{"deviceName":"Phone","timestamp":123}""",
            LumeTransportCrypto.decrypt(key, envelope, "lume-pair-request-v1"),
        )
    }

    @Test
    fun bindsRealtimeMessagesToTheirStreamAndSequence() {
        val key = LumeTransportCrypto.keyFromToken("stream-test")
        val aad = "lume-stream-message-v1:stream-a:7"
        val envelope = LumeTransportCrypto.encrypt(
            key,
            """{"type":"hello"}""",
            aad,
            ByteArray(12) { it.toByte() },
        )

        assertEquals(
            """{"type":"hello"}""",
            LumeTransportCrypto.decrypt(key, envelope, aad),
        )
        assertThrows(Exception::class.java) {
            LumeTransportCrypto.decrypt(
                key,
                envelope,
                "lume-stream-message-v1:stream-b:7",
            )
        }
    }
}
