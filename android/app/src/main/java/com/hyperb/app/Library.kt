package com.hyperb.app

import android.content.Context
import org.json.JSONArray
import org.json.JSONObject

/** One address in the history or the favorites. */
data class LibraryEntry(
    val url: String,
    val label: String,
    val at: Long,
    val visits: Int,
)

/**
 * History and favorites, in the same JSON shape as the Rust core's `Library`
 * so that every client stores the same thing.
 */
data class LibraryState(
    val history: List<LibraryEntry> = emptyList(),
    val favorites: List<LibraryEntry> = emptyList(),
) {
    fun isFavorite(url: String): Boolean = favorites.any { it.url == url }

    /** Bumps `url` to the front of the history and counts the visit. */
    fun record(url: String, label: String, now: Long): LibraryState {
        val visits = (history.firstOrNull { it.url == url }?.visits ?: 0) + 1
        val entry = LibraryEntry(url, label, now, visits)
        val rest = history.filterNot { it.url == url }
        return copy(history = (listOf(entry) + rest).take(HISTORY_LIMIT))
    }

    fun favorite(url: String, label: String, now: Long): LibraryState =
        if (isFavorite(url)) this
        else copy(favorites = listOf(LibraryEntry(url, label, now, 0)) + favorites)

    fun unfavorite(url: String): LibraryState =
        copy(favorites = favorites.filterNot { it.url == url })

    fun toggleFavorite(url: String, label: String, now: Long): LibraryState =
        if (isFavorite(url)) unfavorite(url) else favorite(url, label, now)

    fun clearHistory(): LibraryState = copy(history = emptyList())

    fun removeFromHistory(url: String): LibraryState =
        copy(history = history.filterNot { it.url == url })

    fun toJson(): String = JSONObject().apply {
        put("history", JSONArray().apply { history.forEach { put(it.toJson()) } })
        put("favorites", JSONArray().apply { favorites.forEach { put(it.toJson()) } })
    }.toString()

    companion object {
        const val HISTORY_LIMIT = 100

        fun fromJson(text: String?): LibraryState {
            if (text.isNullOrBlank()) return LibraryState()
            return try {
                val o = JSONObject(text)
                LibraryState(
                    history = entries(o.optJSONArray("history")),
                    favorites = entries(o.optJSONArray("favorites")),
                )
            } catch (e: Exception) {
                LibraryState()
            }
        }

        private fun entries(array: JSONArray?): List<LibraryEntry> {
            if (array == null) return emptyList()
            return (0 until array.length()).mapNotNull { i ->
                val o = array.optJSONObject(i) ?: return@mapNotNull null
                val url = o.optString("url").takeIf { it.isNotBlank() } ?: return@mapNotNull null
                LibraryEntry(
                    url = url,
                    label = o.optString("label", url),
                    at = o.optLong("at"),
                    visits = o.optInt("visits"),
                )
            }
        }

        private fun LibraryEntry.toJson(): JSONObject = JSONObject().apply {
            put("url", url)
            put("label", label)
            put("at", at)
            put("visits", visits)
        }
    }
}

/** Persists [LibraryState] in shared preferences. */
class LibraryStore(context: Context) {
    private val prefs = context.getSharedPreferences("hyperb", Context.MODE_PRIVATE)

    fun load(): LibraryState = LibraryState.fromJson(prefs.getString(KEY, null))

    fun save(library: LibraryState) {
        prefs.edit().putString(KEY, library.toJson()).apply()
    }

    private companion object {
        const val KEY = "library"
    }
}
