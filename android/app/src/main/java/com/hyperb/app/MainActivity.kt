package com.hyperb.app

import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.util.Log
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawingPadding
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Lock
import androidx.compose.material.icons.filled.Refresh
import androidx.compose.material.icons.filled.Warning
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
import androidx.compose.ui.unit.dp
import kotlin.concurrent.thread

/** Local demo servers (`just demoserver`, reached through `adb reverse tcp:9001 tcp:9001`). */
private val SUGGESTIONS =
    listOf("localhost:9001/restaurant", "localhost:9001/therapist", "localhost:9001/textstats")

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
        intent?.dataString?.let(browser::navigate)
        setContent { HyperbTheme { BrowserScreen(browser) } }
    }

    override fun onDestroy() {
        browser.session?.cancel()
        super.onDestroy()
    }
}

/** The address bar and the current session. All state is read and written on the main thread. */
class Browser : Host {
    var publicKey: String? = null
    var address by mutableStateOf("")
    var snapshot by mutableStateOf<Snapshot?>(null)
        private set
    var error by mutableStateOf<String?>(null)
        private set
    var session: Session? = null
        private set
    private val main = Handler(Looper.getMainLooper())

    override val connected get() = snapshot?.connected == true

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
        snapshot = s.snapshot()
        snapshot?.let { address = it.url }
        thread(name = "hyperb-session") {
            s.run { snap -> main.post { if (session === s) snapshot = snap } }
        }
    }

    fun reload() = navigate(snapshot?.url ?: address)

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
    Column(
        Modifier.fillMaxSize().padding(32.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.spacedBy(12.dp, Alignment.CenterVertically),
    ) {
        Text("hyperb", style = MaterialTheme.typography.displaySmall, fontWeight = FontWeight.ExtraBold)
        Text(
            "Enter the address of a hypereffect server. http:// and https:// are used as given; " +
                "without a scheme, local hosts use http:// and others https://.",
            textAlign = TextAlign.Center,
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        for (s in SUGGESTIONS) TextButton(onClick = { b.navigate(s) }) { Text(s) }
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
