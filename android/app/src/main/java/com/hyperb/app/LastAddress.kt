package com.hyperb.app

import android.content.Context

/**
 * The address of the last session that opened, kept in shared preferences so
 * that a restart — or a process Android killed and later revived — reconnects
 * to it instead of showing the welcome screen.
 */
class LastAddress(context: Context) {
    private val prefs = context.getSharedPreferences("hyperb", Context.MODE_PRIVATE)

    fun load(): String? = prefs.getString(KEY, null)?.takeIf { it.isNotBlank() }

    /** Called on every successful navigation; the write may be asynchronous. */
    fun save(address: String) {
        prefs.edit().putString(KEY, address).apply()
    }

    /** Like [save], but waits for the write: for when the process may die next. */
    fun saveNow(address: String) {
        prefs.edit().putString(KEY, address).commit()
    }

    private companion object {
        const val KEY = "address"
    }
}
