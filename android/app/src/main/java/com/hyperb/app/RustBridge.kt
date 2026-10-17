package com.hyperb.app

import android.content.Context

/** The Rust core (`crates/mobile`). See [Session] for the handle rules. */
object RustBridge {
    init {
        System.loadLibrary("hyperb_mobile")
    }

    /** Sets up logging, TLS and the identity kept in [filesDir]; returns its public key. */
    @JvmStatic external fun init(context: Context, filesDir: String): String

    /** Starts a session; throws [RuntimeException] if the address is invalid. */
    @JvmStatic external fun open(address: String): Long
    @JvmStatic external fun cancel(handle: Long)
    @JvmStatic external fun free(handle: Long)

    /** Blocks until network events have been applied; false once the session is over. */
    @JvmStatic external fun await(handle: Long): Boolean
    @JvmStatic external fun snapshot(handle: Long): String

    /** Input on element [id]: [value] (JSON) goes to its bound signal, then [event] runs. */
    @JvmStatic external fun input(handle: Long, id: String, value: String?, event: String?)
}
