package expo.modules.relaywatch

import android.content.Intent
import androidx.core.content.ContextCompat
import expo.modules.kotlin.modules.Module
import expo.modules.kotlin.modules.ModuleDefinition

/**
 * Starts and stops the service that watches running chats while the app is
 * in the background (see WatchService).
 */
class RelayWatchModule : Module() {
  override fun definition() = ModuleDefinition {
    Name("RelayWatch")

    // Returns false when Android doesn't allow starting it right now (for
    // example while the app is in the background).
    Function("start") { baseUrl: String, token: String, scheme: String, sessionId: String? ->
      val ctx = appContext.reactContext ?: return@Function false
      val intent = Intent(ctx, WatchService::class.java)
        .putExtra(WatchService.EXTRA_URL, baseUrl)
        .putExtra(WatchService.EXTRA_TOKEN, token)
        .putExtra(WatchService.EXTRA_SCHEME, scheme)
        .putExtra(WatchService.EXTRA_SESSION, sessionId)
      try {
        ContextCompat.startForegroundService(ctx, intent)
        true
      } catch (e: Exception) {
        false
      }
    }

    Function("stop") {
      val ctx = appContext.reactContext ?: return@Function null
      ctx.stopService(Intent(ctx, WatchService::class.java))
      null
    }

    // The chat on screen right now (no notification is needed for it), or
    // null when none is or the app is in the background.
    Function("setViewing") { sessionId: String? ->
      WatchService.viewing = sessionId
      null
    }
  }
}
