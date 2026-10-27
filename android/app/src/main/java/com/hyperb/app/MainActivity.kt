package com.hyperb.app

import android.content.Intent
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.util.Log
import androidx.activity.ComponentActivity
import androidx.activity.compose.BackHandler
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.compose.foundation.clickable
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawingPadding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Close
import androidx.compose.material.icons.filled.Favorite
import androidx.compose.material.icons.filled.FavoriteBorder
import androidx.compose.material.icons.filled.Home
import androidx.compose.material.icons.filled.Lock
import androidx.compose.material.icons.filled.Refresh
import androidx.compose.material.icons.filled.Warning
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.ListItem
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.dynamicDarkColorScheme
import androidx.compose.material3.dynamicLightColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import kotlin.concurrent.thread

class MainActivity : ComponentActivity() {
    private val browser = Browser()

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge()
        browser.publicKey = try {
            RustBridge.init(applicationContext, filesDir.absolutePath)
        } catch (e: Throwable) {
            Log.e("hyperb", "Rust core failed to start", e)
            null
        }
        browser.loadLibrary()
        // A link opens what it names; otherwise the last session comes back,
        // also after a restart or a rotation.
        if (savedInstanceState == null && intent?.action == Intent.ACTION_VIEW) {
            openLink(intent)
        } else {
            browser.library.session.firstOrNull()?.let { browser.navigate(it) }
        }
        setContent { HyperbTheme { BrowserScreen(browser) } }
    }

    /** With `singleTask`, links that arrive while the app runs come here. */
    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        setIntent(intent)
        openLink(intent)
    }

    private fun openLink(intent: Intent?) {
        if (intent?.action != Intent.ACTION_VIEW) return
        intent.dataString?.let { browser.navigate(it, link = true) }
    }

    override fun onDestroy() {
        browser.session?.cancel()
        super.onDestroy()
    }
}

/**
 * The address bar, the current session, and the history and favorites. All state is read and
 * written on the main thread.
 */
class Browser : Host {
    var publicKey: String? = null
    var address by mutableStateOf("")
    var library by mutableStateOf(Library.EMPTY)
        private set
    var snapshot by mutableStateOf<Snapshot?>(null)
        private set
    var error by mutableStateOf<String?>(null)
        private set
    var session: Session? = null
        private set
    private val main = Handler(Looper.getMainLooper())

    override val connected get() = snapshot?.connected == true

    /** Opens what the user typed or picked, or a [link] from another app. */
    fun navigate(input: String, link: Boolean = false, reload: Boolean = false) {
        val s = try {
            Session(input, link, reload)
        } catch (e: RuntimeException) {
            error = e.message
            return
        }
        if (!reload) loadLibrary()
        session?.cancel()
        session = s
        error = null
        snapshot = s.snapshot()
        snapshot?.let {
            address = it.address
            try {
                RustBridge.setSession(it.address)
            } catch (e: RuntimeException) {
                Log.e("hyperb", "session", e)
            }
        }
        thread(name = "hyperb-session") {
            s.run { snap -> main.post { if (session === s) snapshot = snap } }
        }
    }

    fun reload() = navigate(snapshot?.url ?: address, reload = snapshot != null)

    /** Ends the session and goes back to the welcome page, with the history and favorites. */
    fun home() {
        session?.cancel()
        session = null
        snapshot = null
        address = ""
        error = null
    }

    fun loadLibrary() {
        library = bridge { RustBridge.library() } ?: return
    }

    fun setFavorite(address: String, on: Boolean) {
        library = bridge { RustBridge.setFavorite(address, on) } ?: return
    }

    /** Removes [address] from the history, or clears it if null. */
    fun forget(address: String?) {
        library = bridge { RustBridge.forget(address) } ?: return
    }

    private fun bridge(call: () -> String): Library? = try {
        Library.parse(call())
    } catch (e: Throwable) {
        Log.e("hyperb", "library", e)
        null
    }

    override fun input(id: String, value: String?, event: String?) {
        val s = session ?: return
        s.input(id, value, event)
        s.snapshot()?.let { snapshot = it }
    }
}

@Composable
fun HyperbTheme(content: @Composable () -> Unit) {
    val dark = isSystemInDarkTheme()
    val context = LocalContext.current
    val colors = when {
        android.os.Build.VERSION.SDK_INT >= 31 ->
            if (dark) dynamicDarkColorScheme(context) else dynamicLightColorScheme(context)
        dark -> darkColorScheme()
        else -> lightColorScheme()
    }
    MaterialTheme(colorScheme = colors, content = content)
}

@Composable
fun BrowserScreen(b: Browser) {
    BackHandler(enabled = b.snapshot != null) { b.home() }
    Surface(Modifier.fillMaxSize()) {
        Column(Modifier.safeDrawingPadding().imePadding()) {
            Toolbar(b)
            val snap = b.snapshot
            if (snap?.state == "connecting") LinearProgressIndicator(Modifier.fillMaxWidth())
            b.error?.let { Banner(it, null) }
            if (snap?.state == "closed") {
                Banner(snap.reason?.let { "Disconnected: $it" } ?: "Disconnected. The server closed the connection.", b::reload)
            }
            if (snap == null) Welcome(b) else Page(snap.tree, b, Modifier.fillMaxSize())
        }
    }
}

@Composable
private fun Toolbar(b: Browser) {
    Row(
        Modifier.fillMaxWidth().padding(horizontal = 8.dp, vertical = 6.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        val snap = b.snapshot
        if (snap != null) {
            IconButton(onClick = b::home) { Icon(Icons.Default.Home, "Favorites and history") }
        }
        OutlinedTextField(
            value = b.address,
            onValueChange = { b.address = it },
            modifier = Modifier.weight(1f),
            singleLine = true,
            placeholder = { Text("Server address") },
            leadingIcon = when {
                snap?.state != "open" -> null
                snap.url.startsWith("https:") -> { { Icon(Icons.Default.Lock, "Connected (encrypted)") } }
                else -> { { Icon(Icons.Default.Warning, "Connected (not encrypted)") } }
            },
            keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Uri, imeAction = ImeAction.Go, autoCorrectEnabled = false),
            keyboardActions = KeyboardActions(onGo = { b.navigate(b.address) }),
        )
        if (snap != null) {
            val favorite = b.library.isFavorite(snap.address)
            IconButton(onClick = { b.setFavorite(snap.address, !favorite) }) {
                if (favorite) {
                    Icon(Icons.Default.Favorite, "Remove from favorites", tint = MaterialTheme.colorScheme.primary)
                } else {
                    Icon(Icons.Default.FavoriteBorder, "Add to favorites")
                }
            }
        }
        IconButton(onClick = b::reload, enabled = snap != null || b.address.isNotBlank()) {
            Icon(Icons.Default.Refresh, "Reconnect")
        }
    }
}

@Composable
private fun Banner(text: String, onReconnect: (() -> Unit)?) {
    Surface(color = MaterialTheme.colorScheme.errorContainer, modifier = Modifier.fillMaxWidth()) {
        Row(Modifier.padding(horizontal = 16.dp, vertical = 6.dp), verticalAlignment = Alignment.CenterVertically) {
            Text(text, Modifier.weight(1f), color = MaterialTheme.colorScheme.onErrorContainer)
            onReconnect?.let { TextButton(onClick = it) { Text("Reconnect") } }
        }
    }
}

@Composable
private fun Welcome(b: Browser) {
    val lib = b.library
    LazyColumn(
        Modifier.fillMaxSize().padding(horizontal = 16.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        item {
            Column(
                Modifier.padding(top = 32.dp, bottom = 8.dp),
                horizontalAlignment = Alignment.CenterHorizontally,
                verticalArrangement = Arrangement.spacedBy(12.dp),
            ) {
                Text("hyperb", style = MaterialTheme.typography.displaySmall, fontWeight = FontWeight.ExtraBold)
                Text(
                    "Enter the address of a hypereffect server, e.g. hyperb://example.com/app. " +
                        "Remote servers use https; localhost, IP addresses and .onion use http.",
                    textAlign = TextAlign.Center,
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
        }
        if (lib.favorites.isNotEmpty()) {
            item { SectionTitle("Favorites") }
            items(lib.favorites, key = { "f:" + it.address }) { e ->
                EntryRow(e, Icons.Default.Favorite, "Remove from favorites", { b.navigate(e.address) }) {
                    b.setFavorite(e.address, false)
                }
            }
        }
        if (lib.history.isNotEmpty()) {
            item {
                SectionTitle("History") {
                    TextButton(onClick = { b.forget(null) }) { Text("Clear history") }
                }
            }
            items(lib.history.take(50), key = { "h:" + it.address }) { e ->
                EntryRow(e, null, "Remove from history", { b.navigate(e.address) }) { b.forget(e.address) }
            }
        }
        b.publicKey?.let {
            item {
                Column(
                    Modifier.padding(vertical = 24.dp),
                    horizontalAlignment = Alignment.CenterHorizontally,
                ) {
                    Text("Your key", style = MaterialTheme.typography.labelMedium)
                    Text(
                        it,
                        style = MaterialTheme.typography.bodySmall,
                        fontFamily = FontFamily.Monospace,
                        textAlign = TextAlign.Center,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
            }
        }
    }
}

@Composable
private fun SectionTitle(text: String, action: @Composable () -> Unit = {}) {
    Row(Modifier.fillMaxWidth().padding(top = 12.dp), verticalAlignment = Alignment.CenterVertically) {
        Text(text, Modifier.weight(1f), style = MaterialTheme.typography.titleMedium)
        action()
    }
}

@Composable
private fun EntryRow(
    e: Library.Entry,
    icon: androidx.compose.ui.graphics.vector.ImageVector?,
    removeLabel: String,
    onOpen: () -> Unit,
    onRemove: () -> Unit,
) {
    Column {
        ListItem(
            headlineContent = { Text(e.title, maxLines = 1, overflow = TextOverflow.Ellipsis) },
            supportingContent = { Text(e.address, maxLines = 1, overflow = TextOverflow.Ellipsis) },
            leadingContent = icon?.let { { Icon(it, null, Modifier.size(20.dp)) } },
            trailingContent = {
                IconButton(onClick = onRemove) { Icon(Icons.Default.Close, removeLabel) }
            },
            modifier = Modifier.fillMaxWidth().clickable(onClick = onOpen),
        )
        HorizontalDivider()
    }
}
