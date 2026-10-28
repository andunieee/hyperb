package com.hyperb.app

import org.json.JSONArray
import org.json.JSONObject

/**
 * One session with a hypereffect server, run by the Rust core.
 *
 * [run] blocks a background thread until the session is over and then frees
 * the native handle; every other call takes [lock] and does nothing after that.
 */
class Session(address: String, link: Boolean, reload: Boolean) {
    private var handle = RustBridge.open(address, link, reload)
    private val lock = Any()

    fun snapshot(): Snapshot? = synchronized(lock) {
        if (handle == 0L) null else Snapshot.parse(RustBridge.snapshot(handle))
    }

    fun input(id: String, value: String?, event: String?) = synchronized(lock) {
        if (handle != 0L) RustBridge.input(handle, id, value, event)
    }

    fun cancel() = synchronized(lock) {
        if (handle != 0L) RustBridge.cancel(handle)
    }

    fun run(onUpdate: (Snapshot) -> Unit) {
        val h = synchronized(lock) { handle }
        if (h == 0L) return
        while (RustBridge.await(h)) {
            snapshot()?.let(onUpdate)
        }
        synchronized(lock) {
            RustBridge.free(h)
            handle = 0L
        }
    }
}

class Snapshot(
    val url: String,
    /** The user-facing `hyperb://` form of [url], for the address bar. */
    val address: String,
    val label: String,
    /** The name the app gives itself, or else [label]. */
    val title: String,
    /** The app's icon, if it sent one; only `data:` URLs are shown. */
    val icon: String?,
    /** `connecting`, `open` or `closed`. */
    val state: String,
    val reason: String?,
    /** Controls are enabled only while connected. */
    val connected: Boolean,
    val tree: List<Node>,
) {
    companion object {
        fun parse(json: String): Snapshot {
            val o = JSONObject(json)
            return Snapshot(
                url = o.getString("url"),
                address = o.optString("address", o.getString("url")),
                label = o.getString("label"),
                title = o.optString("title", o.getString("label")),
                icon = o.opt("icon") as? String,
                state = o.getString("state"),
                reason = o.opt("reason") as? String,
                connected = o.optBoolean("connected"),
                tree = Node.list(o.optJSONArray("tree")),
            )
        }
    }
}

/** The history (most recent first) and favorites, from [RustBridge.library]. */
class Library(val history: List<Entry>, val favorites: List<Entry>) {
    class Entry(val address: String, val title: String, val icon: String?)

    fun isFavorite(address: String?) = favorites.any { it.address == address }

    companion object {
        val EMPTY = Library(emptyList(), emptyList())

        fun parse(json: String): Library {
            val o = JSONObject(json)
            fun list(key: String): List<Entry> {
                val a = o.optJSONArray(key) ?: return emptyList()
                return List(a.length()) {
                    val e = a.getJSONObject(it)
                    Entry(e.getString("address"), e.optString("title", e.getString("address")), e.opt("icon") as? String)
                }
            }
            return Library(list("history"), list("favorites"))
        }
    }
}

/** A resolved element: attributes are already evaluated, and `$value` holds the bound signal. */
class Node(val id: String, val type: String, val attrs: JSONObject, val children: List<Node>) {
    fun str(k: String): String? = attrs.opt(k) as? String
    fun bool(k: String): Boolean = attrs.opt(k) == true
    fun num(k: String): Double? = (attrs.opt(k) as? Number)?.toDouble()
    fun has(k: String): Boolean = attrs.has(k) && !attrs.isNull(k)
    val visible get() = bool("visible")
    val bound: Any? get() = attrs.opt("\$value").takeUnless { it == JSONObject.NULL }

    companion object {
        fun list(a: JSONArray?): List<Node> =
            if (a == null) emptyList() else List(a.length()) { parse(a.getJSONObject(it)) }

        private fun parse(o: JSONObject) = Node(
            id = o.getString("id"),
            type = o.getString("type"),
            attrs = o.optJSONObject("attrs") ?: JSONObject(),
            children = list(o.optJSONArray("children")),
        )
    }
}

/** JSON for a value written to a bound signal. */
object Json {
    fun str(s: String): String = JSONObject.quote(s)
    fun bool(b: Boolean): String = b.toString()
    fun num(v: Double?): String = when {
        v == null || !v.isFinite() -> "null"
        v == Math.rint(v) && Math.abs(v) < 9e15 -> v.toLong().toString()
        else -> v.toString()
    }
}
