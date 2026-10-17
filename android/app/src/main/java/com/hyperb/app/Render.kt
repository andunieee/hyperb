@file:OptIn(ExperimentalLayoutApi::class, ExperimentalMaterial3Api::class)

package com.hyperb.app

import android.graphics.BitmapFactory
import android.util.Base64
import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.RowScope
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.selection.selectable
import androidx.compose.foundation.selection.toggleable
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.CheckCircle
import androidx.compose.material.icons.filled.Close
import androidx.compose.material.icons.filled.Info
import androidx.compose.material.icons.filled.Warning
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.Checkbox
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.ExposedDropdownMenuBox
import androidx.compose.material3.ExposedDropdownMenuDefaults
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.MenuAnchorType
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedCard
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.RadioButton
import androidx.compose.material3.ScrollableTabRow
import androidx.compose.material3.SegmentedButton
import androidx.compose.material3.SegmentedButtonDefaults
import androidx.compose.material3.SingleChoiceSegmentedButtonRow
import androidx.compose.material3.Slider
import androidx.compose.material3.Surface
import androidx.compose.material3.Switch
import androidx.compose.material3.Tab
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.VerticalDivider
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.MutableState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.key
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.focus.onFocusChanged
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.TextRange
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.text.input.TextFieldValue
import androidx.compose.ui.text.input.VisualTransformation
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import kotlinx.coroutines.delay
import java.net.URLDecoder

/*
 * Draws a resolved hypereffect tree with Compose. The Rust client keeps the
 * tree, the signals and the expressions; every input goes back to it through
 * [Host.input], and the next snapshot redraws the page.
 */

interface Host {
    /** Controls are disabled while the session is not connected. */
    val connected: Boolean

    /** Input on element [id]: [value] (JSON) goes to its bound signal, then [event] runs. */
    fun input(id: String, value: String?, event: String?)
}

/** How long an input waits after the last keystroke before running onChange. */
private const val CHANGE_IDLE_MS = 700L

@Composable
fun Page(tree: List<Node>, host: Host, modifier: Modifier = Modifier) {
    Column(modifier.verticalScroll(rememberScrollState()).padding(16.dp)) {
        Stack(null, tree, host)
        Spacer(Modifier.height(32.dp))
    }
}

@Composable
private fun Element(n: Node, h: Host, modifier: Modifier = Modifier) {
    when (n.type) {
        "column" -> Stack(n, n.children, h, modifier = modifier)
        "tab" -> Stack(n, n.children, h, modifier = modifier.padding(top = 4.dp))
        "row" -> RowEl(n, h, modifier)
        "card" -> CardEl(n, h, modifier)
        "list" -> ListEl(n, h, modifier)
        "tabs" -> TabsEl(n, h, modifier)
        "dialog" -> DialogEl(n, h)
        "text" -> TextEl(n, modifier)
        "notice" -> NoticeEl(n, modifier)
        "badge" -> BadgeEl(n, modifier)
        "image" -> ImageEl(n, modifier)
        "divider" -> HorizontalDivider(modifier)
        "progress" -> ProgressEl(n, modifier)
        "button" -> ButtonEl(n, h, modifier)
        "input" -> InputEl(n, h, modifier)
        "checkbox" -> CheckboxEl(n, h, modifier)
        "select" -> SelectEl(n, h, modifier)
        "slider" -> SliderEl(n, h, modifier)
    }
}

// --- containers --------------------------------------------------------------

private fun gap(name: String?, default: String): Dp = when (name ?: default) {
    "none" -> 0.dp
    "small" -> 6.dp
    "large" -> 24.dp
    else -> 12.dp
}

/** A column of children; [n] holds the layout attributes (null for the root). */
@Composable
private fun Stack(n: Node?, kids: List<Node>, h: Host, defaultGap: String = "medium", modifier: Modifier = Modifier) {
    val align = n?.str("align") ?: "stretch"
    Column(modifier, verticalArrangement = Arrangement.spacedBy(gap(n?.str("gap"), defaultGap))) {
        for (k in kids) {
            if (!k.visible) continue
            val shrinks = k.type == "badge" || (k.type == "image" && k.str("variant") != "full")
            val m = when {
                k.type == "divider" -> Modifier.fillMaxWidth()
                align == "center" -> Modifier.align(Alignment.CenterHorizontally)
                align == "end" -> Modifier.align(Alignment.End)
                align == "start" || shrinks -> Modifier.align(Alignment.Start)
                else -> Modifier.fillMaxWidth()
            }
            key(k.id) { Element(k, h, m) }
        }
    }
}

@Composable
private fun RowEl(n: Node, h: Host, modifier: Modifier) {
    val g = gap(n.str("gap"), "small")
    val arrangement = when (n.str("justify")) {
        "center" -> Arrangement.spacedBy(g, Alignment.CenterHorizontally)
        "end" -> Arrangement.spacedBy(g, Alignment.End)
        "between" -> Arrangement.SpaceBetween
        else -> Arrangement.spacedBy(g)
    }
    val v = when (n.str("align")) {
        "start", "stretch" -> Alignment.Top
        "end" -> Alignment.Bottom
        else -> Alignment.CenterVertically
    }
    val kids = n.children.filter { it.visible }
    val grows = { k: Node -> k.type == "input" || k.type == "slider" }
    val content: @Composable RowScope.() -> Unit = {
        for (k in kids) {
            key(k.id) {
                val m = Modifier.align(v).let { if (grows(k)) it.weight(1f) else it }
                if (k.type == "divider") VerticalDivider(m.height(24.dp)) else Element(k, h, m)
            }
        }
    }
    // Rows with a growing control fill one line; others wrap on narrow screens.
    if (kids.any(grows)) {
        Row(modifier, horizontalArrangement = arrangement, content = content)
    } else {
        FlowRow(modifier, horizontalArrangement = arrangement, verticalArrangement = Arrangement.spacedBy(g)) { content() }
    }
}

@Composable
private fun CardEl(n: Node, h: Host, modifier: Modifier) {
    OutlinedCard(modifier) {
        Column(Modifier.padding(horizontal = 16.dp, vertical = 14.dp), verticalArrangement = Arrangement.spacedBy(10.dp)) {
            n.str("title")?.takeIf { it.isNotEmpty() }?.let {
                Text(it, style = MaterialTheme.typography.titleMedium, fontWeight = FontWeight.Bold)
            }
            Stack(n, n.children, h)
        }
    }
}

@Composable
private fun ListEl(n: Node, h: Host, modifier: Modifier) {
    val items = n.children.filter { it.visible }
    Surface(
        modifier,
        shape = RoundedCornerShape(10.dp),
        border = BorderStroke(1.dp, MaterialTheme.colorScheme.outlineVariant),
    ) {
        Column {
            items.forEachIndexed { i, k ->
                if (i > 0) HorizontalDivider(color = MaterialTheme.colorScheme.outlineVariant)
                key(k.id) { Element(k, h, Modifier.fillMaxWidth().padding(horizontal = 12.dp, vertical = 10.dp)) }
            }
            val empty = n.str("empty")
            if (items.isEmpty() && !empty.isNullOrEmpty()) {
                Text(empty, Modifier.padding(12.dp), color = MaterialTheme.colorScheme.onSurfaceVariant)
            }
        }
    }
}

@Composable
private fun TabsEl(n: Node, h: Host, modifier: Modifier) {
    val shown = n.children.filter { it.visible }
    var selected by rememberSynced(n.id, n.bound as? String)
    val current = shown.firstOrNull { it.id == selected } ?: shown.firstOrNull()
    Column(modifier, verticalArrangement = Arrangement.spacedBy(10.dp)) {
        if (current != null) {
            ScrollableTabRow(selectedTabIndex = shown.indexOf(current), edgePadding = 0.dp) {
                for (t in shown) {
                    Tab(
                        selected = t === current,
                        onClick = {
                            selected = t.id
                            h.input(n.id, Json.str(t.id), "onChange")
                        },
                        text = { Text(t.str("label") ?: t.id) },
                    )
                }
            }
            key(current.id) { Element(current, h, Modifier.fillMaxWidth()) }
        }
    }
}

@Composable
private fun DialogEl(n: Node, h: Host) {
    if (!(n.visible && n.bool("open"))) return
    val dismissable = n.has("onDismiss")
    val dismiss = { h.input(n.id, null, "onDismiss") }
    Dialog(
        onDismissRequest = { if (dismissable) dismiss() },
        properties = DialogProperties(dismissOnBackPress = dismissable, dismissOnClickOutside = dismissable),
    ) {
        Surface(shape = RoundedCornerShape(12.dp), tonalElevation = 6.dp) {
            Column(Modifier.padding(horizontal = 20.dp, vertical = 18.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
                val title = n.str("title")?.takeIf { it.isNotEmpty() }
                if (title != null || dismissable) {
                    Row(verticalAlignment = Alignment.CenterVertically) {
                        Text(
                            title ?: "",
                            Modifier.weight(1f),
                            style = MaterialTheme.typography.titleLarge,
                            fontWeight = FontWeight.Bold,
                        )
                        if (dismissable) {
                            IconButton(onClick = dismiss) { Icon(Icons.Default.Close, "Close") }
                        }
                    }
                }
                Column(Modifier.weight(1f, fill = false).verticalScroll(rememberScrollState())) {
                    Stack(n, n.children, h)
                }
            }
        }
    }
}

// --- content -------------------------------------------------------------------

@Composable
private fun TextEl(n: Node, modifier: Modifier) {
    val t = MaterialTheme.typography
    val variant = n.str("variant")
    val style: TextStyle = when (variant) {
        "h1" -> t.headlineMedium.copy(fontWeight = FontWeight.ExtraBold)
        "h2" -> t.headlineSmall.copy(fontWeight = FontWeight.Bold)
        "h3" -> t.titleLarge.copy(fontWeight = FontWeight.Bold)
        "caption" -> t.bodySmall.copy(color = MaterialTheme.colorScheme.onSurfaceVariant)
        "code" -> t.bodyMedium.copy(fontFamily = FontFamily.Monospace)
        else -> t.bodyLarge
    }
    var m = modifier
    if (variant in setOf("h1", "h2", "h3")) m = m.semantics { heading() }
    if (variant == "code") {
        m = m.clip(RoundedCornerShape(6.dp))
            .background(MaterialTheme.colorScheme.onSurface.copy(alpha = 0.06f))
            .padding(horizontal = 8.dp, vertical = 6.dp)
    }
    SelectionContainer(m) { Text(n.str("text") ?: "", style = style) }
}

/** The color for a status variant (`info`, `success`, `warning`, `failure`). */
@Composable
private fun statusColor(variant: String?): Color {
    val dark = isSystemInDarkTheme()
    return when (variant) {
        "success" -> if (dark) Color(0xFF81C784) else Color(0xFF2E7D32)
        "warning" -> if (dark) Color(0xFFFFB74D) else Color(0xFFB26A00)
        "failure" -> MaterialTheme.colorScheme.error
        "info" -> MaterialTheme.colorScheme.primary
        else -> MaterialTheme.colorScheme.onSurface
    }
}

@Composable
private fun NoticeEl(n: Node, modifier: Modifier) {
    val variant = n.str("variant") ?: "info"
    val c = statusColor(variant)
    val icon = when (variant) {
        "success" -> Icons.Default.CheckCircle
        "warning", "failure" -> Icons.Default.Warning
        else -> Icons.Default.Info
    }
    val shape = RoundedCornerShape(8.dp)
    Row(
        modifier.clip(shape).background(c.copy(alpha = 0.1f)).border(1.dp, c.copy(alpha = 0.5f), shape)
            .padding(horizontal = 12.dp, vertical = 10.dp),
        horizontalArrangement = Arrangement.spacedBy(10.dp),
    ) {
        Icon(icon, null, tint = c)
        Column(verticalArrangement = Arrangement.spacedBy(2.dp)) {
            n.str("title")?.takeIf { it.isNotEmpty() }?.let { Text(it, fontWeight = FontWeight.Bold) }
            n.str("text")?.takeIf { it.isNotEmpty() }?.let { Text(it) }
        }
    }
}

@Composable
private fun BadgeEl(n: Node, modifier: Modifier) {
    val variant = n.str("variant") ?: "neutral"
    val c = statusColor(variant)
    Text(
        n.str("text") ?: "",
        modifier.border(1.dp, c.copy(alpha = if (variant == "neutral") 0.25f else 0.6f), CircleShape)
            .background(c.copy(alpha = 0.06f), CircleShape)
            .padding(horizontal = 8.dp, vertical = 1.dp),
        color = c,
        style = MaterialTheme.typography.labelMedium,
    )
}

@Composable
private fun ImageEl(n: Node, modifier: Modifier) {
    val src = n.str("src") ?: ""
    val bitmap = remember(src) { decodeDataUri(src) }
    val alt = n.str("alt")
    if (bitmap == null) {
        Text(alt ?: "", modifier, color = MaterialTheme.colorScheme.onSurfaceVariant)
        return
    }
    val m = when (n.str("variant")) {
        "thumbnail" -> modifier.size(64.dp).clip(RoundedCornerShape(4.dp))
        "avatar" -> modifier.size(40.dp).clip(CircleShape)
        "full" -> modifier.fillMaxWidth()
        else -> modifier
    }
    val crop = n.str("variant") in setOf("thumbnail", "avatar")
    Image(bitmap, alt, m, contentScale = if (crop) ContentScale.Crop else ContentScale.Fit)
}

/** Decodes `data:` images; other schemes are not fetched, so the alt text shows. */
private fun decodeDataUri(src: String): ImageBitmap? = runCatching {
    if (!src.startsWith("data:")) return null
    val comma = src.indexOf(',')
    if (comma < 0) return null
    val meta = src.substring(5, comma)
    val data = src.substring(comma + 1)
    val bytes = if (meta.endsWith(";base64")) {
        Base64.decode(data.filterNot { it.isWhitespace() }, Base64.DEFAULT)
    } else {
        URLDecoder.decode(data.replace("+", "%2B"), "UTF-8").toByteArray()
    }
    BitmapFactory.decodeByteArray(bytes, 0, bytes.size)?.asImageBitmap()
}.getOrNull()

@Composable
private fun ProgressEl(n: Node, modifier: Modifier) {
    Column(modifier, verticalArrangement = Arrangement.spacedBy(6.dp)) {
        n.str("label")?.takeIf { it.isNotEmpty() }?.let { Text(it) }
        val value = n.num("value")
        when {
            n.str("variant") == "spinner" -> CircularProgressIndicator(Modifier.size(24.dp), strokeWidth = 3.dp)
            value == null -> LinearProgressIndicator(Modifier.fillMaxWidth())
            else -> LinearProgressIndicator(progress = { value.toFloat().coerceIn(0f, 1f) }, Modifier.fillMaxWidth())
        }
    }
}

// --- controls ------------------------------------------------------------------

private fun Host.enabled(n: Node) = connected && !n.bool("disabled")

/** Local state that follows [value] whenever it changes, for unbound controls. */
@Composable
private fun <T> rememberSynced(id: String, value: T): MutableState<T> {
    val s = remember(id) { mutableStateOf(value) }
    LaunchedEffect(value) { s.value = value }
    return s
}

@Composable
private fun ButtonEl(n: Node, h: Host, modifier: Modifier) {
    val enabled = h.enabled(n)
    val onClick = { h.input(n.id, null, "onPress") }
    val label: @Composable () -> Unit = { Text(n.str("label") ?: "") }
    when (val v = n.str("variant")) {
        "primary" -> Button(onClick, modifier, enabled) { label() }
        "failure", "success" -> {
            val c = statusColor(v)
            val colors = ButtonDefaults.buttonColors(
                containerColor = c,
                contentColor = if (v == "failure") MaterialTheme.colorScheme.onError else Color.White,
            )
            Button(onClick, modifier, enabled, colors = colors) { label() }
        }
        "plain" -> TextButton(onClick, modifier, enabled) { label() }
        else -> OutlinedButton(onClick, modifier, enabled) { label() }
    }
}

private fun parseNumber(s: String): Double? = s.trim().toDoubleOrNull()?.takeIf { it.isFinite() }

@Composable
private fun InputEl(n: Node, h: Host, modifier: Modifier) {
    val variant = n.str("variant") ?: "text"
    val number = variant == "number"
    val boundNumber = (n.bound as? Number)?.toDouble()
    val boundText = if (number) boundNumber?.let(Json::num) ?: "" else n.bound as? String ?: ""
    var field by remember(n.id) { mutableStateOf(TextFieldValue(boundText, TextRange(boundText.length))) }
    var pending by remember(n.id) { mutableStateOf(false) }
    // Changes made by the server or by expressions replace the text.
    LaunchedEffect(boundText) {
        val same = if (number) parseNumber(field.text) == boundNumber else field.text == boundText
        if (n.has("bind") && !same) field = TextFieldValue(boundText, TextRange(boundText.length))
    }
    val fireChange = {
        if (pending) {
            pending = false
            h.input(n.id, null, "onChange")
        }
    }
    LaunchedEffect(field.text, pending) {
        if (pending) {
            delay(CHANGE_IDLE_MS)
            fireChange()
        }
    }
    val error = n.str("error")?.takeIf { it.isNotEmpty() }
    val multiline = variant == "multiline"
    OutlinedTextField(
        value = field,
        onValueChange = { new ->
            val edited = new.text != field.text
            field = new
            if (edited) {
                h.input(n.id, if (number) Json.num(parseNumber(new.text)) else Json.str(new.text), null)
                pending = true
            }
        },
        modifier = modifier.onFocusChanged { if (!it.isFocused) fireChange() },
        enabled = h.enabled(n),
        label = n.str("label")?.takeIf { it.isNotEmpty() }?.let { { Text(it) } },
        placeholder = n.str("placeholder")?.let { { Text(it) } },
        isError = error != null,
        supportingText = error?.let { { Text(it) } },
        singleLine = !multiline,
        minLines = if (multiline) 3 else 1,
        visualTransformation = if (variant == "password") PasswordVisualTransformation() else VisualTransformation.None,
        keyboardOptions = KeyboardOptions(
            keyboardType = when (variant) {
                "password" -> KeyboardType.Password
                "number" -> KeyboardType.Decimal
                "email" -> KeyboardType.Email
                else -> KeyboardType.Text
            },
            imeAction = if (multiline) ImeAction.Default else ImeAction.Done,
        ),
        keyboardActions = KeyboardActions(onDone = {
            fireChange()
            h.input(n.id, null, "onSubmit")
        }),
    )
}

@Composable
private fun CheckboxEl(n: Node, h: Host, modifier: Modifier) {
    var checked by rememberSynced(n.id, n.bound == true)
    val switch = n.str("variant") == "switch"
    val enabled = h.enabled(n)
    Row(
        modifier.toggleable(checked, enabled = enabled, role = if (switch) Role.Switch else Role.Checkbox) {
            checked = it
            h.input(n.id, Json.bool(it), "onChange")
        }.padding(vertical = 4.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        if (switch) {
            Switch(checked, null, enabled = enabled)
            Spacer(Modifier.width(12.dp))
        } else {
            Checkbox(checked, null, enabled = enabled)
            Spacer(Modifier.width(8.dp))
        }
        Text(n.str("label") ?: "")
    }
}

@Composable
private fun SelectEl(n: Node, h: Host, modifier: Modifier) {
    val options = n.attrs.optJSONArray("options").let { a ->
        if (a == null) emptyList() else (0 until a.length()).mapNotNull { i ->
            val o = a.optJSONObject(i) ?: return@mapNotNull null
            val value = o.opt("value") as? String ?: return@mapNotNull null
            val label = o.opt("label") as? String ?: return@mapNotNull null
            value to label
        }
    }
    var selected by rememberSynced(n.id, n.bound as? String)
    val enabled = h.enabled(n)
    val choose = { v: String ->
        selected = v
        h.input(n.id, Json.str(v), "onChange")
    }
    val label = n.str("label")?.takeIf { it.isNotEmpty() }
    Column(modifier, verticalArrangement = Arrangement.spacedBy(6.dp)) {
        when (n.str("variant")) {
            "radio" -> {
                label?.let { Text(it) }
                for ((value, text) in options) {
                    Row(
                        Modifier.fillMaxWidth().selectable(value == selected, enabled = enabled, role = Role.RadioButton) {
                            choose(value)
                        },
                        verticalAlignment = Alignment.CenterVertically,
                    ) {
                        RadioButton(value == selected, null, enabled = enabled)
                        Spacer(Modifier.width(8.dp))
                        Text(text)
                    }
                }
            }
            "segmented" -> {
                label?.let { Text(it) }
                SingleChoiceSegmentedButtonRow {
                    options.forEachIndexed { i, (value, text) ->
                        SegmentedButton(
                            selected = value == selected,
                            onClick = { choose(value) },
                            shape = SegmentedButtonDefaults.itemShape(i, options.size),
                            enabled = enabled,
                        ) { Text(text) }
                    }
                }
            }
            else -> {
                var expanded by remember { mutableStateOf(false) }
                ExposedDropdownMenuBox(expanded, { expanded = it && enabled }) {
                    OutlinedTextField(
                        value = options.firstOrNull { it.first == selected }?.second ?: "",
                        onValueChange = {},
                        readOnly = true,
                        enabled = enabled,
                        label = label?.let { { Text(it) } },
                        trailingIcon = { ExposedDropdownMenuDefaults.TrailingIcon(expanded) },
                        modifier = Modifier.menuAnchor(MenuAnchorType.PrimaryNotEditable).fillMaxWidth(),
                    )
                    ExposedDropdownMenu(expanded, { expanded = false }) {
                        for ((value, text) in options) {
                            DropdownMenuItem(text = { Text(text) }, onClick = {
                                expanded = false
                                choose(value)
                            })
                        }
                    }
                }
            }
        }
    }
}

private fun decimals(step: Double): Int = step.toBigDecimal().stripTrailingZeros().scale().coerceAtLeast(0)

@Composable
private fun SliderEl(n: Node, h: Host, modifier: Modifier) {
    val min = n.num("min") ?: 0.0
    val max = maxOf(n.num("max") ?: 100.0, min)
    val step = n.num("step")?.takeIf { it > 0 } ?: 1.0
    val places = decimals(step)
    var value by rememberSynced(n.id, ((n.bound as? Number)?.toDouble() ?: min).coerceIn(min, max))
    var pending by remember(n.id) { mutableStateOf(false) }
    Column(modifier) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Text(n.str("label") ?: "", Modifier.weight(1f))
            Text("%.${places}f".format(value), color = MaterialTheme.colorScheme.onSurfaceVariant)
        }
        // Values snap to the step here, so the track needs no tick marks.
        Slider(
            value = value.toFloat(),
            onValueChange = { raw ->
                val snapped = (min + Math.round((raw - min) / step) * step)
                    .toBigDecimal().setScale(places, java.math.RoundingMode.HALF_UP).toDouble()
                if (snapped != value) {
                    value = snapped
                    h.input(n.id, Json.num(snapped), null)
                    pending = true
                }
            },
            onValueChangeFinished = {
                if (pending) {
                    pending = false
                    h.input(n.id, null, "onChange")
                }
            },
            valueRange = min.toFloat()..max.toFloat(),
            enabled = h.enabled(n),
        )
    }
}
