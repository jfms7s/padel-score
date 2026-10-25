package io.github.jfms7s.padelscore.haptics

import android.content.Context
import android.os.Build
import android.os.VibrationEffect
import android.os.Vibrator
import android.os.VibratorManager

/** Three buzzes the player can tell apart without looking: short, long, double. */
interface Haptics {
    fun accepted()
    fun notSent()
    fun refused()
}

class VibratorHaptics(context: Context) : Haptics {
    // VibratorManager is API 31; minSdk 30 (Wear OS 3) still needs the plain Vibrator service.
    private val vibrator: Vibrator? = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
        context.getSystemService(VibratorManager::class.java)?.defaultVibrator
    } else {
        @Suppress("DEPRECATION")
        context.getSystemService(Vibrator::class.java)
    }

    override fun accepted() = vibrate(VibrationEffect.createPredefined(VibrationEffect.EFFECT_CLICK))

    override fun notSent() = vibrate(VibrationEffect.createOneShot(400, VibrationEffect.DEFAULT_AMPLITUDE))

    override fun refused() = vibrate(VibrationEffect.createWaveform(longArrayOf(0, 120, 80, 120), -1))

    private fun vibrate(effect: VibrationEffect) {
        vibrator?.takeIf { it.hasVibrator() }?.vibrate(effect)
    }
}
