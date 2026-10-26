package io.github.jfms7s.padelscore

import android.content.Intent
import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.lifecycle.lifecycleScope
import kotlinx.coroutines.launch

class MainActivity : ComponentActivity() {
    private var restarting = false

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        val host = application as ContainerHost
        val container = host.container
        setContent { WatchApp(container, host.settingsStore, host.local, onSettingsApplied = ::restart) }
    }

    /**
     * New server settings: rebuild the container, then swap this activity for a fresh one. Finishing
     * clears this activity's ViewModelStore, so every screen's model (and any sign-in still polling
     * the old server) is cancelled, and the new activity starts from scratch on the new container.
     */
    private fun restart() {
        if (restarting) return
        restarting = true
        lifecycleScope.launch {
            (application as ContainerHost).reload()
            startActivity(Intent(this@MainActivity, MainActivity::class.java))
            finish()
        }
    }
}
