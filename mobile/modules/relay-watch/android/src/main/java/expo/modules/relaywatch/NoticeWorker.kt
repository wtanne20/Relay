package expo.modules.relaywatch

import android.content.Context
import androidx.work.Constraints
import androidx.work.ExistingPeriodicWorkPolicy
import androidx.work.NetworkType
import androidx.work.PeriodicWorkRequestBuilder
import androidx.work.WorkManager
import androidx.work.Worker
import androidx.work.WorkerParameters
import okhttp3.OkHttpClient
import okhttp3.Request
import org.json.JSONObject
import java.util.concurrent.TimeUnit

/**
 * Checks in with Relay on the PC every 15 minutes or so (Android decides
 * exactly when), even with the app closed, and notifies about tasks that
 * finished or need approval since the last check: scheduled and queued
 * tasks that started while nobody had the app open. The PC keeps a short
 * log of those (`GET /notices`) with names and status only.
 */
class NoticeWorker(ctx: Context, params: WorkerParameters) : Worker(ctx, params) {
  companion object {
    private const val NAME = "relay-notices"
    private const val KEY_URL = "url"
    private const val KEY_TOKEN = "token"
    private const val KEY_SCHEME = "scheme"
    private const val KEY_LAST = "last_notice"

    /** Starts (or updates) the periodic check-in. */
    fun schedule(ctx: Context, url: String, token: String, scheme: String) {
      val p = Notifier.prefs(ctx)
      val changed = p.getString(KEY_URL, null) != url
      val e = p.edit().putString(KEY_URL, url).putString(KEY_TOKEN, token).putString(KEY_SCHEME, scheme)
      // Another PC: start from its newest notice instead of an old id.
      if (changed) e.remove(KEY_LAST)
      e.apply()
      val req = PeriodicWorkRequestBuilder<NoticeWorker>(15, TimeUnit.MINUTES)
        .setConstraints(Constraints.Builder().setRequiredNetworkType(NetworkType.CONNECTED).build())
        .build()
      WorkManager.getInstance(ctx).enqueueUniquePeriodicWork(NAME, ExistingPeriodicWorkPolicy.UPDATE, req)
    }

    fun cancel(ctx: Context) {
      WorkManager.getInstance(ctx).cancelUniqueWork(NAME)
      Notifier.prefs(ctx).edit().remove(KEY_TOKEN).remove(KEY_LAST).apply()
    }
  }

  private val http = OkHttpClient.Builder()
    .connectTimeout(15, TimeUnit.SECONDS)
    .readTimeout(20, TimeUnit.SECONDS)
    .build()

  override fun doWork(): Result {
    val ctx = applicationContext
    val p = Notifier.prefs(ctx)
    val url = p.getString(KEY_URL, null)?.trimEnd('/') ?: return Result.success()
    val token = p.getString(KEY_TOKEN, null) ?: return Result.success()
    val scheme = p.getString(KEY_SCHEME, null) ?: "relay"
    val last = p.getLong(KEY_LAST, -1L)
    val query = if (last < 0) "" else "?after=$last&limit=50"
    val req = Request.Builder()
      .url("$url/notices$query")
      .header("Authorization", "Bearer $token")
      .build()
    try {
      http.newCall(req).execute().use { res ->
        // An older Relay on the PC (404) or a revoked token: try again later.
        if (!res.isSuccessful) return Result.success()
        val body = JSONObject(res.body?.string() ?: "{}")
        val list = body.optJSONArray("notices")
        var newest = body.optLong("last_id", last)
        if (last >= 0 && list != null) {
          for (i in 0 until list.length()) {
            val n = list.optJSONObject(i) ?: continue
            newest = maxOf(newest, n.optLong("id", newest))
            val sid = n.optString("session_id")
            val status = n.optString("status")
            if (sid.isEmpty() || Notifier.recentlyTold(ctx, sid, status)) continue
            Notifier.tell(ctx, scheme, sid, n.optString("project_name"), n.optString("title"), status)
          }
        }
        p.edit().putLong(KEY_LAST, newest).apply()
      }
    } catch (e: Exception) {
      // The PC is asleep or out of reach; the next check catches up.
    }
    return Result.success()
  }
}
