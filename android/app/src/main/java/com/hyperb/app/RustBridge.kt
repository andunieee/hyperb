package com.hyperb.app

import android.content.Context

/** The Rust core (`crates/mobile`). See [Session] for the handle rules. */
object RustBridge {
    init {
        System.loadLibrary("hyperb_mobile")
    }

    /** Sets up logging, TLS and the identity kept in [filesDir]; returns its public key. */
    @JvmStatic external fun init(context: Context, filesDir: String): String

    /**
     * Starts a session, which goes to the history unless [reload]. A [link] (from another app)
     * cannot open local files. Throws [RuntimeException] if the address is invalid.
     */
    @JvmStatic external fun open(address: String, link: Boolean, reload: Boolean): Long
    @JvmStatic external fun cancel(handle: Long)
    @JvmStatic external fun free(handle: Long)

    /** Blocks until network events have been applied; false once the session is over. */
    @JvmStatic external fun await(handle: Long): Boolean
    @JvmStatic external fun snapshot(handle: Long): String

    /** Input on element [id]: [value] (JSON) goes to its bound signal, then [event] runs. */
    @JvmStatic external fun input(handle: Long, id: String, value: String?, event: String?)

    /** The history and favorites, as JSON; see [Library]. */
    @JvmStatic external fun library(): String

    /** The `https` icon at [src], downloaded unless it is kept already; blocks. */
    @JvmStatic external fun icon(src: String): ByteArray?

    /** Adds or removes a favorite (a wasm app is then kept for offline use); returns the library. */
    @JvmStatic external fun setFavorite(address: String, on: Boolean): String

    /** Removes [address] from the history, or clears it if null; returns the library. */
    @JvmStatic external fun forget(address: String?): String
}
