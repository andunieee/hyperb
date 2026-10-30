package com.hyperb.app

import android.app.ActivityManager
import android.content.ActivityNotFoundException
import android.content.Intent
import android.net.Uri
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.util.Log
import android.widget.Toast
import androidx.activity.ComponentActivity
import androidx.activity.compose.BackHandler
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.Image
import androidx.compose.foundation.border
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.selection.selectable
import androidx.compose.foundation.selection.toggleable
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.material.icons.filled.Check
import androidx.compose.material3.Switch
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.graphics.luminance
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.AlertDialogDefaults
import androidx.compose.material3.SegmentedButton
import androidx.compose.material3.SegmentedButtonDefaults
import androidx.compose.material3.SingleChoiceSegmentedButtonRow
import androidx.compose.ui.semantics.Role
import androidx.compose.foundation.clickable
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
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Close
import androidx.compose.material.icons.filled.Favorite
import androidx.compose.material.icons.filled.FavoriteBorder
import androidx.compose.material.icons.filled.Home
import androidx.compose.material.icons.filled.Lock
import androidx.compose.material.icons.filled.Refresh
import androidx.compose.material.icons.filled.Settings
import androidx.compose.material.icons.filled.Warning
import androidx.compose.material3.AlertDialog
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
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.produceState
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.text.LinkAnnotation
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.buildAnnotatedString
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.withLink
import androidx.compose.ui.text.withStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import com.journeyapps.barcodescanner.ScanContract
import com.journeyapps.barcodescanner.ScanOptions
import kotlin.concurrent.thread
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext

/**
 * A window: the launcher opens one on the welcome page, and every link opens another
 * ([LinkActivity]), each with its own session. They share the process and the Rust core.
 */
open class MainActivity : ComponentActivity() {
    private val browser = Browser()

    /** A scanned code is opened like a link from another app: it cannot open local files. */
    private fun openScanned(text: String?) {
        if (!text.isNullOrBlank()) browser.navigate(text.trim(), link = true)
    }

    private val scanApp = registerForActivityResult(ActivityResultContracts.StartActivityForResult()) {
        if (it.resultCode == RESULT_OK) openScanned(it.data?.getStringExtra("SCAN_RESULT"))
    }
    private val scanBuiltIn = registerForActivityResult(ScanContract()) { openScanned(it.contents) }

    /**
     * Scans a QR code with a scanner app (Binary Eye, ZXing Barcode Scanner, ...) through the
     * ZXing intent, or with the built-in scanner if none is installed.
     */
    private fun scan() {
        try {
            scanApp.launch(Intent("com.google.zxing.client.android.SCAN").putExtra("SCAN_MODE", "QR_CODE_MODE"))
        } catch (_: ActivityNotFoundException) {
            scanBuiltIn.launch(
                ScanOptions()
                    .setDesiredBarcodeFormats(ScanOptions.QR_CODE)
                    .setPrompt("Scan a hyperb QR code")
                    .setBeepEnabled(false)
                    .setOrientationLocked(false),
            )
        }
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge()
        Appearance.load(this)
        browser.publicKey = try {
            RustBridge.init(applicationContext, filesDir.absolutePath)
        } catch (e: Throwable) {
            Log.e("hyperb", "Rust core failed to start", e)
            null
        }
        browser.loadLibrary()
        browser.openUrl = ::openUrl
        // A window that the system recreates goes back to its address; a new one opens its
        // link, if it has one.
        val restored = savedInstanceState?.getString(ADDRESS)
        when {
            restored != null -> browser.navigate(restored, reload = true)
            savedInstanceState == null && intent?.action == Intent.ACTION_VIEW ->
                intent.dataString?.let { browser.navigate(it, link = true) }
        }
        setContent {
            val title = browser.snapshot?.title ?: "hyperb"
            val icon = browser.snapshot?.icon
            LaunchedEffect(title, icon) {
                val bitmap = icon?.let { withContext(Dispatchers.IO) { loadIcon(it) } }
                @Suppress("DEPRECATION")
                setTaskDescription(ActivityManager.TaskDescription(title, bitmap))
            }
            HyperbTheme { BrowserScreen(browser, ::scan) }
        }
    }

    /**
     * Opens a URL a button asked for: a hyperb address in a window of its own, like a link from
     * another app, and anything else in the app that handles it (a web browser, a mail app).
     */
    private fun openUrl(open: Open) {
        val uri = Uri.parse(open.url)
        if (open.app) {
            startActivity(Intent(Intent.ACTION_VIEW, uri, this, LinkActivity::class.java))
            return
        }
        try {
            startActivity(Intent(Intent.ACTION_VIEW, uri).addCategory(Intent.CATEGORY_BROWSABLE))
        } catch (_: ActivityNotFoundException) {
            Toast.makeText(this, "No app can open ${open.url}", Toast.LENGTH_SHORT).show()
        }
    }

    /** Other windows may have changed the history and favorites. */
    override fun onResume() {
        super.onResume()
        browser.loadLibrary()
    }

    override fun onSaveInstanceState(outState: Bundle) {
        super.onSaveInstanceState(outState)
        browser.snapshot?.let { outState.putString(ADDRESS, it.address) }
    }

    override fun onDestroy() {
        browser.session?.cancel()
        super.onDestroy()
    }

    private companion object {
        const val ADDRESS = "address"
    }
}

/** A window for one link, in a task of its own (`documentLaunchMode="always"`). */
class LinkActivity : MainActivity()

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
    /** Opens a URL a button asked for, outside the app. */
    var openUrl: (Open) -> Unit = {}
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
        snapshot?.let { address = it.address }
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
        val opens = s.input(id, value, event)
        s.snapshot()?.let { snapshot = it }
        opens.forEach(openUrl)
    }
}

@Composable
fun BrowserScreen(b: Browser, onScan: () -> Unit) {
    BackHandler(enabled = b.snapshot != null) { b.home() }
    Surface(Modifier.fillMaxSize()) {
        Column(Modifier.safeDrawingPadding().imePadding()) {
            Toolbar(b, onScan)
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
private fun Toolbar(b: Browser, onScan: () -> Unit) {
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
            trailingIcon = {
                IconButton(onClick = onScan) { Icon(painterResource(R.drawable.ic_qr_scan), "Scan a QR code") }
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
        var settings by remember { mutableStateOf(false) }
        IconButton(onClick = { settings = true }) { Icon(Icons.Default.Settings, "Settings") }
        if (settings) Settings(b.publicKey) { settings = false }
    }
}

@Composable
private fun Settings(publicKey: String?, onDismiss: () -> Unit) {
    val look = LocalLook.current
    AlertDialog(
        onDismissRequest = onDismiss,
        confirmButton = { TextButton(onClick = onDismiss, shape = look.buttonShape) { Text("Close") } },
        modifier = look.border()?.let { Modifier.border(it, MaterialTheme.shapes.extraLarge) } ?: Modifier,
        tonalElevation = if (look.outlined) 0.dp else AlertDialogDefaults.TonalElevation,
        title = { Text("Settings") },
        text = {
            Column(Modifier.verticalScroll(rememberScrollState()), verticalArrangement = Arrangement.spacedBy(4.dp)) {
                Heading("Theme", top = false)
                Choice(Mode.entries, Appearance.mode, Mode::label) { Appearance.mode = it }
                Heading("Color · ${Appearance.palette.label}")
                Swatches()
                Toggle("Vivid colors", Appearance.vivid, enabled = Appearance.palette != Palette.MONO) { Appearance.vivid = it }
                Toggle("Black background", Appearance.black, enabled = Appearance.mode != Mode.LIGHT) { Appearance.black = it }
                Heading("Contrast")
                Choice(Contrast.entries, Appearance.contrast, Contrast::label) { Appearance.contrast = it }
                Heading("Corners")
                Choice(Corners.entries, Appearance.corners, Corners::label) { Appearance.corners = it }
                Heading("Size")
                Choice(Size.entries, Appearance.size, Size::label) { Appearance.size = it }
                Toggle("Solid borders", Appearance.borders) { Appearance.borders = it }
                Text("Your key", Modifier.padding(top = 12.dp), style = MaterialTheme.typography.labelMedium)
                SelectionContainer {
                    Text(
                        publicKey ?: "unavailable",
                        style = MaterialTheme.typography.bodySmall,
                        fontFamily = FontFamily.Monospace,
                    )
                }
            }
        },
    )
}

@Composable
private fun Heading(text: String, top: Boolean = true) {
    Text(text, Modifier.padding(top = if (top) 12.dp else 0.dp, bottom = 4.dp), style = MaterialTheme.typography.labelMedium)
}

@Composable
private fun <T> Choice(options: List<T>, selected: T, label: (T) -> String, onSelect: (T) -> Unit) {
    SingleChoiceSegmentedButtonRow(Modifier.fillMaxWidth()) {
        options.forEachIndexed { i, option ->
            SegmentedButton(
                selected = option == selected,
                onClick = { onSelect(option) },
                shape = SegmentedButtonDefaults.itemShape(i, options.size, LocalLook.current.buttonShape),
            ) { Text(label(option), maxLines = 1) }
        }
    }
}

@Composable
private fun Toggle(text: String, checked: Boolean, enabled: Boolean = true, onChange: (Boolean) -> Unit) {
    Row(
        Modifier.fillMaxWidth().toggleable(checked, enabled, Role.Switch, onChange).padding(vertical = 4.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Text(
            text,
            Modifier.weight(1f),
            color = MaterialTheme.colorScheme.onSurface.copy(alpha = if (enabled) 1f else 0.38f),
        )
        Switch(checked, null, enabled = enabled)
    }
}

/** A circle of each palette's seed color, checked when selected. */
@OptIn(ExperimentalLayoutApi::class)
@Composable
private fun Swatches() {
    val shape = LocalLook.current.buttonShape
    val outline = MaterialTheme.colorScheme.onSurface
    FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
        for (palette in Palette.entries) {
            val color = palette.seed ?: systemSeed()
            val selected = palette == Appearance.palette
            Box(
                Modifier.size(40.dp).clip(shape).background(color)
                    .then(if (selected) Modifier.border(2.dp, outline, shape) else Modifier)
                    .selectable(selected, role = Role.RadioButton) { Appearance.palette = palette }
                    .semantics { contentDescription = palette.label },
                contentAlignment = Alignment.Center,
            ) {
                when {
                    selected -> Icon(Icons.Default.Check, null, tint = if (color.luminance() > 0.4f) Color.Black else Color.White)
                    palette == Palette.SYSTEM -> Text("A", color = Color.White, fontWeight = FontWeight.Bold)
                }
            }
        }
    }
}

@Composable
private fun Banner(text: String, onReconnect: (() -> Unit)?) {
    val look = LocalLook.current
    val c = MaterialTheme.colorScheme
    Surface(
        color = if (look.outlined) c.surface else c.errorContainer,
        border = look.border()?.let { BorderStroke(it.width, c.error) },
        modifier = Modifier.fillMaxWidth(),
    ) {
        Row(Modifier.padding(horizontal = 16.dp, vertical = 6.dp), verticalAlignment = Alignment.CenterVertically) {
            Text(text, Modifier.weight(1f), color = if (look.outlined) c.error else c.onErrorContainer)
            onReconnect?.let { TextButton(onClick = it, shape = look.buttonShape) { Text("Reconnect") } }
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
                    buildAnnotatedString {
                        append("A browser for ")
                        withLink(LinkAnnotation.Url("https://github.com/hypereffect/hyperb")) {
                            withStyle(SpanStyle(color = MaterialTheme.colorScheme.primary, fontWeight = FontWeight.SemiBold)) {
                                append("hyperb")
                            }
                        }
                        append(" apps.")
                    },
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
                    TextButton(onClick = { b.forget(null) }, shape = LocalLook.current.buttonShape) { Text("Clear history") }
                }
            }
            items(lib.history.take(50), key = { "h:" + it.address }) { e ->
                EntryRow(e, null, "Remove from history", { b.navigate(e.address) }) { b.forget(e.address) }
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
    // The app's icon, or the list's.
    val bitmap = produceState<ImageBitmap?>(null, e.icon) {
        value = e.icon?.let { withContext(Dispatchers.IO) { loadIcon(it) } }?.asImageBitmap()
    }.value
    Column {
        ListItem(
            headlineContent = { Text(e.title, maxLines = 1, overflow = TextOverflow.Ellipsis) },
            supportingContent = { Text(e.address, maxLines = 1, overflow = TextOverflow.Ellipsis) },
            leadingContent = when {
                bitmap != null -> { { Image(bitmap, null, Modifier.size(20.dp)) } }
                icon != null -> { { Icon(icon, null, Modifier.size(20.dp)) } }
                else -> null
            },
            trailingContent = {
                IconButton(onClick = onRemove) { Icon(Icons.Default.Close, removeLabel) }
            },
            modifier = Modifier.fillMaxWidth().clickable(onClick = onOpen),
        )
        HorizontalDivider()
    }
}
