package com.hyperb.app

import android.content.Context
import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.CornerBasedShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Shapes
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.dynamicDarkColorScheme
import androidx.compose.material3.dynamicLightColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.unit.Density
import androidx.compose.ui.unit.dp

/** What [MaterialTheme] does not cover: the shape of buttons and badges, and outlines instead of tones. */
class Look(val buttonShape: CornerBasedShape, val outlined: Boolean) {
    /** The outline of dialogs and banners, which are tinted instead unless [outlined]. */
    @Composable
    fun border(): BorderStroke? = if (outlined) BorderStroke(1.dp, MaterialTheme.colorScheme.outline) else null
}

val LocalLook = staticCompositionLocalOf { Style.DEFAULT.look }

private fun corners(vararg dp: Int) = dp.map { RoundedCornerShape(it.dp) }.let { Shapes(it[0], it[1], it[2], it[3], it[4]) }

enum class Style(val label: String, val shapes: Shapes, val look: Look, val scale: Float = 1f) {
    DEFAULT("Default", Shapes(), Look(CircleShape, outlined = false)),

    /** Everything 10% smaller, so more fits on the screen. */
    COMPACT("Compact", corners(4, 6, 8, 12, 20), Look(CircleShape, outlined = false), scale = 0.9f),
    OUTLINED("Outlined", corners(4, 4, 6, 8, 12), Look(RoundedCornerShape(6.dp), outlined = true)),
    SQUARE("Square", corners(0, 0, 0, 0, 0), Look(RoundedCornerShape(0.dp), outlined = false)),
}

enum class Palette(val label: String) {
    /** The wallpaper's colors on Android 12 and later, Material's baseline before. */
    SYSTEM("System"),
    BLUE("Blue"),
    MONO("Mono"),
}

/** The style and palette, shared by all windows and kept in the preferences. */
object Appearance {
    var style by mutableStateOf(Style.DEFAULT)
        private set
    var palette by mutableStateOf(Palette.SYSTEM)
        private set
    private var context: Context? = null

    private val prefs get() = context?.getSharedPreferences("appearance", Context.MODE_PRIVATE)

    fun load(context: Context) {
        if (this.context != null) return
        this.context = context.applicationContext
        val p = prefs ?: return
        style = Style.entries.firstOrNull { it.name == p.getString("style", null) } ?: style
        palette = Palette.entries.firstOrNull { it.name == p.getString("palette", null) } ?: palette
    }

    fun set(style: Style = this.style, palette: Palette = this.palette) {
        this.style = style
        this.palette = palette
        prefs?.edit()?.putString("style", style.name)?.putString("palette", palette.name)?.apply()
    }
}

@Composable
fun HyperbTheme(content: @Composable () -> Unit) {
    val dark = isSystemInDarkTheme()
    val context = LocalContext.current
    val colors = when (Appearance.palette) {
        Palette.BLUE -> if (dark) Palettes.blueDark else Palettes.blueLight
        Palette.MONO -> if (dark) Palettes.monoDark else Palettes.monoLight
        Palette.SYSTEM -> when {
            android.os.Build.VERSION.SDK_INT >= 31 ->
                if (dark) dynamicDarkColorScheme(context) else dynamicLightColorScheme(context)
            dark -> darkColorScheme()
            else -> lightColorScheme()
        }
    }
    val style = Appearance.style
    val density = LocalDensity.current
    CompositionLocalProvider(
        LocalLook provides style.look,
        LocalDensity provides Density(density.density * style.scale, density.fontScale),
    ) {
        MaterialTheme(colorScheme = colors, shapes = style.shapes, content = content)
    }
}
