package expo.modules.relaywatch

import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.net.Uri
import android.os.Build
import androidx.core.app.NotificationCompat

/**
 * Shows "finished / failed / needs your approval" notifications for both the
 * live watcher (WatchService) and the periodic check-in (NoticeWorker), and
 * remembers what was shown so the two don't tell you the same thing twice.
 */
object Notifier {
  const val CHANNEL_EVENTS = "tasks"
  const val PREFS = "relay_watch"
  /** A notice the watcher already showed within this window isn't repeated. */
  private const val TOLD_WINDOW_MS = 6L * 60 * 60 * 1000

  fun prefs(ctx: Context) = ctx.getSharedPreferences(PREFS, Context.MODE_PRIVATE)

  fun ensureChannel(ctx: Context) {
    if (Build.VERSION.SDK_INT < 26) return
    val m = ctx.getSystemService(Context.NOTIFICATION_SERVICE) as NotificationManager
    if (m.getNotificationChannel(CHANNEL_EVENTS) == null) {
      m.createNotificationChannel(
        NotificationChannel(CHANNEL_EVENTS, "Task updates", NotificationManager.IMPORTANCE_HIGH),
      )
    }
  }

  private fun key(sessionId: String, status: String) = "told:$sessionId:$status"

  /** Whether this was shown recently (by either path). */
  fun recentlyTold(ctx: Context, sessionId: String, status: String): Boolean {
    val at = prefs(ctx).getLong(key(sessionId, status), 0L)
    return System.currentTimeMillis() - at < TOLD_WINDOW_MS
  }

  private fun remember(ctx: Context, sessionId: String, status: String) {
    val p = prefs(ctx)
    val now = System.currentTimeMillis()
    val e = p.edit().putLong(key(sessionId, status), now)
    // Forget old entries so the file stays small.
    for ((k, v) in p.all) {
      if (k.startsWith("told:") && v is Long && now - v > TOLD_WINDOW_MS) e.remove(k)
    }
    e.apply()
  }

  /**
   * Shows one notification per chat and status (a repeat replaces it).
   * [decorate] can add to it, given the builder and the notification's id;
   * the watcher uses it to put Allow / Deny on approval requests.
   */
  fun tell(
    ctx: Context,
    scheme: String,
    sessionId: String,
    project: String,
    title: String,
    status: String,
    decorate: ((NotificationCompat.Builder, Int) -> Unit)? = null,
  ) {
    remember(ctx, sessionId, status)
    if (WatchService.viewing == sessionId) return
    ensureChannel(ctx)
    val label = when (status) {
      "done" -> "finished"
      "error" -> "failed"
      "stopped" -> "was stopped"
      "approval" -> "needs your approval"
      else -> status
    }
    val open = Intent(Intent.ACTION_VIEW, Uri.parse("$scheme://session/${Uri.encode(sessionId)}"))
      .setPackage(ctx.packageName)
      .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_CLEAR_TOP)
    val nid = "$sessionId:$status".hashCode()
    val b = NotificationCompat.Builder(ctx, CHANNEL_EVENTS)
      .setSmallIcon(icon(ctx))
      .setContentTitle(project.ifBlank { "Relay" })
      .setContentText("${title.ifBlank { "A chat" }} $label")
      .setPriority(NotificationCompat.PRIORITY_HIGH)
      .setAutoCancel(true)
      .setContentIntent(
        PendingIntent.getActivity(ctx, sessionId.hashCode(), open, PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT),
      )
    decorate?.invoke(b, nid)
    val m = ctx.getSystemService(Context.NOTIFICATION_SERVICE) as NotificationManager
    try {
      m.notify(nid, b.build())
    } catch (e: SecurityException) {
      // Notifications are turned off for Relay.
    }
  }

  fun icon(ctx: Context): Int {
    val res = ctx.resources.getIdentifier("notification_icon", "drawable", ctx.packageName)
    return if (res != 0) res else ctx.applicationInfo.icon
  }
}
