package com.hyperb.app

import android.content.Context
import android.content.SharedPreferences
import androidx.activity.ComponentActivity
import androidx.activity.SystemBarStyle
import androidx.activity.enableEdgeToEdge
import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.CornerBasedShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.ColorScheme
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Shapes
import androidx.compose.material3.dynamicDarkColorScheme
import androidx.compose.material3.dynamicLightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.toArgb
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.res.colorResource
import androidx.compose.ui.unit.Density
import androidx.compose.ui.unit.dp
import com.materialkolor.PaletteStyle
import com.materialkolor.dynamicColorScheme
import kotlin.properties.ReadWriteProperty
import kotlin.reflect.KProperty

/** What [MaterialTheme] does not cover: the shape of buttons and badges, and outlines instead of tones. */
class Look(val buttonShape: CornerBasedShape, val outlined: Boolean) {
    /** The outline of dialogs and banners, which are tinted instead unless [outlined]. */
    @Composable
    fun border(): BorderStroke? = if (outlined) BorderStroke(1.dp, MaterialTheme.colorScheme.outline) else null
}

val LocalLook = staticCompositionLocalOf { Look(CircleShape, outlined = false) }

private fun corners(vararg dp: Int) = dp.map { RoundedCornerShape(it.dp) }.let { Shapes(it[0], it[1], it[2], it[3], it[4]) }

enum class Mode(val label: String) { SYSTEM("Auto"), LIGHT("Light"), DARK("Dark") }

enum class Corners(val label: String, val shapes: Shapes, val buttonShape: CornerBasedShape) {
    SQUARE("Square", corners(0, 0, 0, 0, 0), RoundedCornerShape(0.dp)),
    ROUNDED("Rounded", corners(4, 4, 6, 8, 12), RoundedCornerShape(6.dp)),
    ROUND("Round", Shapes(), CircleShape),
}

/** How much everything is scaled, so that more or less fits on the screen. */
enum class Size(val label: String, val scale: Float) { COMPACT("Compact", 0.9f), NORMAL("Normal", 1f), LARGE("Large", 1.1f) }

/** The contrast level of the generated color schemes, from -1 to 1. */
enum class Contrast(val label: String, val level: Double) { NORMAL("Normal", 0.0), MEDIUM("Medium", 0.5), HIGH("High", 1.0) }

/** The seed the color scheme is generated from; null for the wallpaper's colors. */
enum class Palette(val label: String, val seed: Color?) {
    SYSTEM("System", null),
    MONO("Mono", Color(0xFF808080)),
    RED("Red", Color(0xFFE53935)),
    ORANGE("Orange", Color(0xFFF57C00)),
    YELLOW("Yellow", Color(0xFFFBC02D)),
    GREEN("Green", Color(0xFF43A047)),
    TEAL("Teal", Color(0xFF00897B)),
    BLUE("Blue", Color(0xFF3366CC)),
    PURPLE("Purple", Color(0xFF7E57C2)),
    PINK("Pink", Color(0xFFD81B60)),
}

/** The appearance settings, shared by all windows and kept in the preferences. */
object Appearance {
    private val all = mutableListOf<Pref<*>>()
    private var context: Context? = null
    private val prefs get() = context?.getSharedPreferences("appearance", Context.MODE_PRIVATE)

    var mode by choice("mode", Mode.SYSTEM)
    var palette by choice("palette", Palette.SYSTEM)
    var contrast by choice("contrast", Contrast.NORMAL)
    var corners by choice("corners", Corners.ROUND)
    var size by choice("size", Size.NORMAL)

    /** More saturated colors than Material's default tonal spot. */
    var vivid by flag("vivid", false)

    /** Solid borders around dialogs, banners and buttons instead of tinted surfaces. */
    var borders by flag("borders", false)

    /** A pure black background in dark mode. */
    var black by flag("black", false)

    fun load(context: Context) {
        if (this.context != null) return
        this.context = context.applicationContext
        val p = prefs ?: return
        all.forEach { it.load(p) }
    }

    private class Pref<T>(val key: String, initial: T, val write: (T) -> String, val read: (String) -> T?) :
        ReadWriteProperty<Any?, T> {
        var state by mutableStateOf(initial)

        override fun getValue(thisRef: Any?, property: KProperty<*>) = state

        override fun setValue(thisRef: Any?, property: KProperty<*>, value: T) {
            state = value
            prefs?.edit()?.putString(key, write(value))?.apply()
        }

        fun load(p: SharedPreferences) {
            p.getString(key, null)?.let(read)?.let { state = it }
        }
    }

    private fun <T> pref(key: String, initial: T, write: (T) -> String, read: (String) -> T?) =
        Pref(key, initial, write, read).also { all += it }

    private inline fun <reified E : Enum<E>> choice(key: String, initial: E) =
        pref(key, initial, { it.name }) { s -> enumValues<E>().firstOrNull { it.name == s } }

    private fun flag(key: String, initial: Boolean) = pref(key, initial, { it.toString() }, String::toBooleanStrictOrNull)
}

/** The wallpaper's accent on Android 12 and later, Material's baseline purple before. */
@Composable
fun systemSeed(): Color =
    if (android.os.Build.VERSION.SDK_INT >= 31) colorResource(android.R.color.system_accent1_500) else Color(0xFF6750A4)

@Composable
fun HyperbTheme(content: @Composable () -> Unit) {
    val a = Appearance
    val dark = when (a.mode) {
        Mode.SYSTEM -> isSystemInDarkTheme()
        Mode.LIGHT -> false
        Mode.DARK -> true
    }
    val context = LocalContext.current
    val seed = a.palette.seed ?: systemSeed()
    val colors = remember(a.palette, seed, dark, a.contrast, a.vivid, a.black) {
        val dynamic = a.palette == Palette.SYSTEM && a.contrast == Contrast.NORMAL && !a.vivid &&
            android.os.Build.VERSION.SDK_INT >= 31
        val style = when {
            a.palette == Palette.MONO -> PaletteStyle.Monochrome
            a.vivid -> PaletteStyle.Vibrant
            else -> PaletteStyle.TonalSpot
        }
        val scheme = when {
            dynamic && dark -> dynamicDarkColorScheme(context)
            dynamic -> dynamicLightColorScheme(context)
            else -> dynamicColorScheme(seed, dark, isAmoled = false, style = style, contrastLevel = a.contrast.level)
        }
        if (dark && a.black) scheme.blackened() else scheme
    }
    SystemBars(dark)
    val density = LocalDensity.current
    CompositionLocalProvider(
        LocalLook provides Look(a.corners.buttonShape, a.borders),
        LocalDensity provides Density(density.density * a.size.scale, density.fontScale),
    ) {
        MaterialTheme(colorScheme = colors, shapes = a.corners.shapes, content = content)
    }
}

private fun ColorScheme.blackened() = copy(
    background = Color.Black,
    surface = Color.Black,
    surfaceDim = Color.Black,
    surfaceContainerLowest = Color.Black,
)

/** Light or dark status and navigation bar icons to match the theme rather than the system. */
@Composable
private fun SystemBars(dark: Boolean) {
    val activity = LocalContext.current as? ComponentActivity ?: return
    DisposableEffect(activity, dark) {
        // The scrims enableEdgeToEdge uses by default on three-button navigation.
        val lightScrim = android.graphics.Color.argb(0xe6, 0xFF, 0xFF, 0xFF)
        val darkScrim = android.graphics.Color.argb(0x80, 0x1b, 0x1b, 0x1b)
        val transparent = Color.Transparent.toArgb()
        activity.enableEdgeToEdge(
            SystemBarStyle.auto(transparent, transparent) { dark },
            SystemBarStyle.auto(lightScrim, darkScrim) { dark },
        )
        onDispose {}
    }
}
