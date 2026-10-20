package com.hyperb.app

import android.content.Intent
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.util.Log
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.compose.foundation.clickable
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawingPadding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Lock
import androidx.compose.material.icons.filled.Refresh
import androidx.compose.material.icons.filled.Warning
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
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
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import kotlin.concurrent.thread

class MainActivity : ComponentActivity() {
    private lateinit var browser: Browser
    private lateinit var lastAddress: LastAddress

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge()
        lastAddress = LastAddress(applicationContext)
        browser = Browser(lastAddress::save, LibraryStore(applicationContext))
        browser.publicKey = try {
            RustBridge.init(applicationContext, filesDir.absolutePath)
        } catch (e: Throwable) {
            Log.e("hyperb", "Rust core failed to start", e)
            null
        }
        // A deep link wins; otherwise reopen what was on screen before the
        // process died, or the last address that opened in an earlier run.
        val initial = intent?.dataString
            ?: savedInstanceState?.getString(STATE_ADDRESS)
            ?: lastAddress.load()
        initial?.let(browser::navigate)
        setContent { HyperbTheme { BrowserScreen(browser) } }
    }

    /**
     * A `hyperb://` or `web+hyperb://` link while the app is already open: the
     * activity is single-top, so the address arrives here instead of onCreate.
     */
    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        setIntent(intent)
        intent.dataString?.let(browser::navigate)
    }

    override fun onSaveInstanceState(outState: Bundle) {
        super.onSaveInstanceState(outState)
        browser.currentAddress()?.let {
            outState.putString(STATE_ADDRESS, it)
            lastAddress.saveNow(it)
        }
    }

    override fun onDestroy() {
        browser.session?.cancel()
        super.onDestroy()
    }

    private companion object {
        const val STATE_ADDRESS = "hyperb.address"
    }
}

/** The address bar, the library and the current session. All state is read and written on the main thread. */
class Browser(
    private val persistAddress: (String) -> Unit = {},
    private val libraryStore: LibraryStore? = null,
) : Host {
    var publicKey: String? = null
    var address by mutableStateOf("")
    /** History and favorites, mirrored to shared preferences. */
    var library by mutableStateOf(libraryStore?.load() ?: LibraryState())
        private set
    /** Whether the history/favorites panel is open. */
    var libraryOpen by mutableStateOf(false)
    var snapshot by mutableStateOf<Snapshot?>(null)
        private set
    var error by mutableStateOf<String?>(null)
        private set
    var session: Session? = null
        private set
    private val main = Handler(Looper.getMainLooper())

    override val connected get() = snapshot?.connected == true

    /** The normalized address of the open session, if any. */
    fun currentAddress(): String? = snapshot?.url

    fun navigate(input: String) {
        val s = try {
            Session(input)
        } catch (e: RuntimeException) {
            error = e.message
            return
        }
        session?.cancel()
        session = s
        error = null
        val snap = s.snapshot()
        snapshot = snap
        snap?.let {
            address = it.address
            persistAddress(it.url)
            updateLibrary(library.record(it.url, it.label, System.currentTimeMillis()))
        }
        thread(name = "hyperb-session") {
            s.run { snap -> main.post { if (session === s) snapshot = snap } }
        }
    }

    fun reload() = navigate(snapshot?.url ?: address)

    /** Stars or unstars the open address. */
    fun toggleFavorite() {
        val snap = snapshot ?: return
        updateLibrary(library.toggleFavorite(snap.url, snap.label, System.currentTimeMillis()))
    }

    fun removeFavorite(url: String) = updateLibrary(library.unfavorite(url))

    fun removeFromHistory(url: String) = updateLibrary(library.removeFromHistory(url))

    fun clearHistory() = updateLibrary(library.clearHistory())

    private fun updateLibrary(next: LibraryState) {
        library = next
        libraryStore?.save(next)
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
    Surface(Modifier.fillMaxSize()) {
        Column(Modifier.safeDrawingPadding().imePadding()) {
            Toolbar(b)
            if (b.libraryOpen) LibraryPanel(b)
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
        IconButton(onClick = b::toggleFavorite, enabled = snap != null) {
            Text(
                if (snap != null && b.library.isFavorite(snap.url)) "★" else "☆",
                style = MaterialTheme.typography.titleLarge,
                color = if (snap != null && b.library.isFavorite(snap.url)) {
                    MaterialTheme.colorScheme.primary
                } else {
                    MaterialTheme.colorScheme.onSurfaceVariant
                },
            )
        }
        IconButton(onClick = { b.libraryOpen = !b.libraryOpen }) {
            Text("☰", style = MaterialTheme.typography.titleLarge)
        }
        IconButton(onClick = b::reload, enabled = snap != null || b.address.isNotBlank()) {
            Icon(Icons.Default.Refresh, "Reconnect")
        }
    }
}

/** The history and favorites, under the toolbar. */
@Composable
private fun LibraryPanel(b: Browser) {
    val library = b.library
    Column(
        Modifier
            .fillMaxWidth()
            .heightIn(max = 300.dp)
            .verticalScroll(rememberScrollState())
            .padding(horizontal = 16.dp, vertical = 6.dp),
    ) {
        Text("FAVORITES", style = MaterialTheme.typography.labelMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
        if (library.favorites.isEmpty()) {
            Text(
                "Star an address to keep it here.",
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                modifier = Modifier.padding(vertical = 6.dp),
            )
        }
        library.favorites.forEach { entry ->
            LibraryRow(
                entry = entry,
                onOpen = {
                    b.navigate(entry.url)
                    b.libraryOpen = false
                },
                onRemove = { b.removeFavorite(entry.url) },
            )
        }
        Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
            Text(
                "HISTORY",
                modifier = Modifier.weight(1f),
                style = MaterialTheme.typography.labelMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
            TextButton(onClick = b::clearHistory) { Text("Clear") }
        }
        if (library.history.isEmpty()) {
            Text(
                "Nothing opened yet.",
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                modifier = Modifier.padding(vertical = 6.dp),
            )
        }
        library.history.forEach { entry ->
            LibraryRow(
                entry = entry,
                onOpen = {
                    b.navigate(entry.url)
                    b.libraryOpen = false
                },
                onRemove = { b.removeFromHistory(entry.url) },
            )
        }
    }
    HorizontalDivider()
}

@Composable
private fun LibraryRow(entry: LibraryEntry, onOpen: () -> Unit, onRemove: () -> Unit) {
    Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
        Column(
            Modifier
                .weight(1f)
                .clickable(onClick = onOpen)
                .padding(vertical = 6.dp),
        ) {
            Text(
                entry.label,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
                fontWeight = FontWeight.SemiBold,
            )
            Text(
                entry.url,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
        TextButton(onClick = onRemove) { Text("✕") }
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
    Column(
        Modifier.fillMaxSize().padding(32.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.spacedBy(12.dp, Alignment.CenterVertically),
    ) {
        Text("hyperb", style = MaterialTheme.typography.displaySmall, fontWeight = FontWeight.ExtraBold)
        Text(
            "Enter the address of a hypereffect server, e.g. hyperb://example.com/app. " +
                "Remote servers use https; localhost, IP addresses and .onion use http.",
            textAlign = TextAlign.Center,
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        b.publicKey?.let {
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
