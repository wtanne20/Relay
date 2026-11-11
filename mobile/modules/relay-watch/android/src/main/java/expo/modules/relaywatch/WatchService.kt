package expo.modules.relaywatch

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Intent
import android.content.pm.ServiceInfo
import android.net.Uri
import android.os.Build
import android.os.Handler
import android.os.HandlerThread
import android.os.IBinder
import androidx.core.app.NotificationCompat
import androidx.core.app.ServiceCompat
import okhttp3.OkHttpClient
import okhttp3.Request
import okhttp3.Response
import okhttp3.WebSocket
import okhttp3.WebSocketListener
import org.json.JSONObject
import java.util.concurrent.TimeUnit

/**
 * Keeps an eye on chats that are working while Relay is in the background,
 * the way a tracker app keeps running: Android allows it because of the
 * ongoing "working" notification. It asks Relay on the PC (over Tailscale,
 * with the app's token) which chats are running, and shows a notification
 * when one finishes, fails, is stopped or needs approval. Only the project
 * name, chat title and status are shown. It stops itself once nothing is
 * running.
 */
class WatchService : Service() {
  companion object {
    const val EXTRA_URL = "url"
    const val EXTRA_TOKEN = "token"
    const val EXTRA_SCHEME = "scheme"
    const val EXTRA_SESSION = "session"
    private const val CHANNEL_WORKING = "relay_working"
    private const val ONGOING_ID = 7301
    private const val POLL_MS = 4000L
    /** Give up after this long without reaching the PC. */
    private const val UNREACHABLE_MS = 30L * 60 * 1000

    @Volatile
    var viewing: String? = null
  }

  private data class Chat(val status: String, val title: String, val project: String)

  private val http = OkHttpClient.Builder()
    .connectTimeout(15, TimeUnit.SECONDS)
    .readTimeout(20, TimeUnit.SECONDS)
    .pingInterval(30, TimeUnit.SECONDS)
    .build()
  private val lock = Any()
  private var thread: HandlerThread? = null
  private var handler: Handler? = null
  private var baseUrl = ""
  private var token = ""
  private var scheme = "relay"
  private val chats = HashMap<String, Chat>()
  private val sockets = HashMap<String, WebSocket>()
  private val approvals = HashSet<String>()
  private var idlePolls = 0
  private var lastReached = System.currentTimeMillis()

  override fun onBind(intent: Intent?): IBinder? = null

  override fun onCreate() {
    super.onCreate()
    channels()
    val t = HandlerThread("relay-watch").also { it.start() }
    thread = t
    handler = Handler(t.looper)
  }

  override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
    val type = if (Build.VERSION.SDK_INT >= 34) ServiceInfo.FOREGROUND_SERVICE_TYPE_REMOTE_MESSAGING else 0
    try {
      ServiceCompat.startForeground(this, ONGOING_ID, ongoing("Watching your chats"), type)
    } catch (e: Exception) {
      stopSelf()
      return START_NOT_STICKY
    }
    val url = intent?.getStringExtra(EXTRA_URL)
    val tok = intent?.getStringExtra(EXTRA_TOKEN)
    if (url.isNullOrBlank() || tok.isNullOrBlank()) {
      if (baseUrl.isEmpty()) stopSelf()
      return START_NOT_STICKY
    }
    synchronized(lock) {
      baseUrl = url.trimEnd('/')
      token = tok
      scheme = intent.getStringExtra(EXTRA_SCHEME) ?: "relay"
      idlePolls = 0
      lastReached = System.currentTimeMillis()
      // The chat that just started counts as running even if it finishes
      // before the first check.
      intent.getStringExtra(EXTRA_SESSION)?.let { id ->
        if (chats[id]?.status != "running") chats[id] = Chat("running", chats[id]?.title ?: "", chats[id]?.project ?: "")
      }
    }
    handler?.removeCallbacksAndMessages(null)
    handler?.post { poll() }
    return START_REDELIVER_INTENT
  }

  override fun onDestroy() {
    handler?.removeCallbacksAndMessages(null)
    thread?.quitSafely()
    synchronized(lock) {
      sockets.values.forEach { it.close(1000, null) }
      sockets.clear()
    }
    super.onDestroy()
  }

  override fun onTimeout(startId: Int) {
    stopSelf()
  }

  private fun poll() {
    val req = Request.Builder()
      .url("$baseUrl/recent?limit=50")
      .header("Authorization", "Bearer $token")
      .build()
    var reached = false
    try {
      http.newCall(req).execute().use { res: Response ->
        if (res.isSuccessful) {
          val body = res.body?.string() ?: ""
          update(JSONObject(body).optJSONArray("sessions"))
          reached = true
        }
      }
    } catch (e: Exception) {
      // The PC is asleep or out of reach; try again shortly.
    }
    val now = System.currentTimeMillis()
    var running: List<Chat> = emptyList()
    synchronized(lock) {
      if (reached) lastReached = now
      running = chats.values.filter { it.status == "running" }
      if (running.isEmpty()) idlePolls++ else idlePolls = 0
    }
    if ((reached && idlePolls >= 2) || now - lastReached > UNREACHABLE_MS) {
      stopSelf()
      return
    }
    val text = when {
      !reached -> "Can't reach your PC right now"
      running.size == 1 -> "Working: ${running[0].title.ifBlank { "a chat" }}"
      else -> "${running.size} chats working"
    }
    manager().notify(ONGOING_ID, ongoing(text))
    handler?.postDelayed({ poll() }, POLL_MS)
  }

  private fun update(list: org.json.JSONArray?) {
    if (list == null) return
    val finished = ArrayList<Pair<String, Chat>>()
    synchronized(lock) {
      for (i in 0 until list.length()) {
        val s = list.optJSONObject(i) ?: continue
        val id = s.optString("id")
        if (id.isEmpty()) continue
        val now = Chat(s.optString("status"), s.optString("title"), s.optString("project_name"))
        val before = chats[id]
        if (before?.status == "running" && now.status != "running") finished.add(id to now)
        chats[id] = now
        if (now.status == "running") watch(id) else sockets.remove(id)?.close(1000, null)
      }
    }
    finished.forEach { (id, c) -> tell(id, c, c.status) }
  }

  /** Opens the chat's live stream to hear about approvals right away. */
  private fun watch(id: String) {
    if (sockets.containsKey(id)) return
    val ws = baseUrl.replaceFirst(Regex("^http", RegexOption.IGNORE_CASE), "ws")
    val req = Request.Builder()
      .url("$ws/sessions/${Uri.encode(id)}/ws")
      .header("Authorization", "Bearer $token")
      .build()
    sockets[id] = http.newWebSocket(req, object : WebSocketListener() {
      override fun onMessage(webSocket: WebSocket, text: String) {
        val ev = try { JSONObject(text) } catch (e: Exception) { return }
        val call = when (ev.optString("type")) {
          "approval_request" -> ev.optJSONObject("approval")?.optString("tool_call_id")
          "hello" -> ev.optJSONObject("pending_approval")?.optString("tool_call_id")
          "status" -> {
            val st = ev.optString("status")
            if (st != "running" && st.isNotEmpty()) {
              var c: Chat? = null
              synchronized(lock) {
                val before = chats[id]
                if (before?.status == "running") {
                  c = before.copy(status = st)
                  chats[id] = c!!
                }
              }
              c?.let { tell(id, it, st) }
            }
            null
          }
          else -> null
        }
        if (call.isNullOrEmpty()) return
        var c: Chat? = null
        synchronized(lock) {
          if (approvals.add(call)) c = chats[id]
        }
        c?.let { tell(id, it, "approval") }
      }

      override fun onFailure(webSocket: WebSocket, t: Throwable, response: Response?) {
        synchronized(lock) {
          if (sockets[id] === webSocket) sockets.remove(id)
        }
      }

      override fun onClosed(webSocket: WebSocket, code: Int, reason: String) {
        synchronized(lock) {
          if (sockets[id] === webSocket) sockets.remove(id)
        }
      }
    })
  }

  private fun tell(id: String, c: Chat, status: String) {
    Notifier.tell(this, scheme, id, c.project, c.title, status)
  }

  private fun ongoing(text: String): Notification {
    val launch = packageManager.getLaunchIntentForPackage(packageName)
    val pi = launch?.let { PendingIntent.getActivity(this, 0, it, PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT) }
    return NotificationCompat.Builder(this, CHANNEL_WORKING)
      .setSmallIcon(icon())
      .setContentTitle("Relay")
      .setContentText(text)
      .setOngoing(true)
      .setOnlyAlertOnce(true)
      .setSilent(true)
      .setPriority(NotificationCompat.PRIORITY_LOW)
      .setContentIntent(pi)
      .build()
  }

  private fun icon(): Int {
    val res = resources.getIdentifier("notification_icon", "drawable", packageName)
    return if (res != 0) res else applicationInfo.icon
  }

  private fun manager() = getSystemService(NOTIFICATION_SERVICE) as NotificationManager

  private fun channels() {
    if (Build.VERSION.SDK_INT < 26) return
    val m = manager()
    if (m.getNotificationChannel(CHANNEL_WORKING) == null) {
      m.createNotificationChannel(
        NotificationChannel(CHANNEL_WORKING, "Chats working", NotificationManager.IMPORTANCE_LOW).apply {
          description = "Shown while Relay watches a chat that is working"
          setShowBadge(false)
        },
      )
    }
    Notifier.ensureChannel(this)
  }
}
