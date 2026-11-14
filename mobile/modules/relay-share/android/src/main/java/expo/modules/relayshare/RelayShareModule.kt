package expo.modules.relayshare

import android.content.Context
import android.content.Intent
import android.net.Uri
import android.provider.OpenableColumns
import androidx.core.content.IntentCompat
import expo.modules.kotlin.modules.Module
import expo.modules.kotlin.modules.ModuleDefinition
import java.io.File

/**
 * Receives what other apps share to Relay through Android's share sheet: a
 * link or text, pictures, screenshots, voice notes or any other file. The
 * intent filters are in app.json (android.intentFilters). Shared files are
 * copied into the app's cache, because the permission to read them ends with
 * the share, and handed to the app as file:// paths.
 */
class RelayShareModule : Module() {
  companion object {
    /** Biggest file copied; matches the server's limit per attachment. */
    const val MAX_BYTES = 25L * 1024 * 1024
    const val MAX_FILES = 16
    private const val TAKEN = "expo.modules.relayshare.TAKEN"

    /** A share that arrived while Relay was already open. */
    @Volatile
    private var pending: Intent? = null
  }

  override fun definition() = ModuleDefinition {
    Name("RelayShare")

    Events("onShare")

    OnNewIntent { intent ->
      if (isShare(intent)) {
        pending = intent
        sendEvent("onShare")
      }
    }

    // The share waiting to be handled, once: null when there is none.
    AsyncFunction("take") {
      val ctx = appContext.reactContext ?: return@AsyncFunction null
      val fromNew = pending
      pending = null
      val intent = fromNew ?: appContext.currentActivity?.intent
      if (intent == null || !isShare(intent) || intent.getBooleanExtra(TAKEN, false)) {
        return@AsyncFunction null
      }
      // Opening Relay again from recent apps brings back the old intent.
      if (fromNew == null && (intent.flags and Intent.FLAG_ACTIVITY_LAUNCHED_FROM_HISTORY) != 0) {
        return@AsyncFunction null
      }
      intent.putExtra(TAKEN, true)
      read(ctx, intent)
    }
  }

  private fun isShare(intent: Intent?): Boolean =
    intent?.action == Intent.ACTION_SEND || intent?.action == Intent.ACTION_SEND_MULTIPLE

  private fun read(ctx: Context, intent: Intent): Map<String, Any?> {
    val text = intent.getCharSequenceExtra(Intent.EXTRA_TEXT)?.toString()
    val subject = intent.getStringExtra(Intent.EXTRA_SUBJECT)

    val uris = LinkedHashSet<Uri>()
    if (intent.action == Intent.ACTION_SEND_MULTIPLE) {
      IntentCompat.getParcelableArrayListExtra(intent, Intent.EXTRA_STREAM, Uri::class.java)?.let { uris.addAll(it) }
    } else {
      IntentCompat.getParcelableExtra(intent, Intent.EXTRA_STREAM, Uri::class.java)?.let { uris.add(it) }
    }
    if (uris.isEmpty()) {
      intent.clipData?.let { clip ->
        for (i in 0 until clip.itemCount) clip.getItemAt(i).uri?.let { uris.add(it) }
      }
    }

    // One share at a time: clear out the files of the last one.
    val dir = File(ctx.cacheDir, "shared")
    dir.deleteRecursively()
    dir.mkdirs()

    val files = ArrayList<Map<String, Any?>>()
    for ((i, uri) in uris.take(MAX_FILES).withIndex()) {
      files.add(copy(ctx, uri, intent.type, File(dir, i.toString())))
    }
    return mapOf(
      "text" to text,
      "subject" to subject,
      "files" to files,
      "dropped" to (uris.size - files.size).coerceAtLeast(0),
    )
  }

  private fun copy(ctx: Context, uri: Uri, fallbackMime: String?, into: File): Map<String, Any?> {
    val resolver = ctx.contentResolver
    var name: String? = null
    var size = -1L
    try {
      resolver.query(uri, arrayOf(OpenableColumns.DISPLAY_NAME, OpenableColumns.SIZE), null, null, null)?.use { c ->
        if (c.moveToFirst()) {
          val n = c.getColumnIndex(OpenableColumns.DISPLAY_NAME)
          val s = c.getColumnIndex(OpenableColumns.SIZE)
          if (n >= 0 && !c.isNull(n)) name = c.getString(n)
          if (s >= 0 && !c.isNull(s)) size = c.getLong(s)
        }
      }
    } catch (_: Exception) {
      // Some apps share plain file or content URIs without these columns.
    }
    val mime = (try { resolver.getType(uri) } catch (_: Exception) { null })
      ?.takeIf { it.isNotBlank() && it != "*/*" }
      ?: fallbackMime?.takeIf { !it.contains('*') }
      ?: "application/octet-stream"
    val display = (name ?: uri.lastPathSegment ?: "file")
      .substringAfterLast('/')
      .filter { it != '\\' && !it.isISOControl() }
      .take(120)
      .let { if (it.isBlank() || it == "." || it == "..") "file" else it }

    val base = mapOf("name" to display, "mime" to mime)
    if (size > MAX_BYTES) return base + mapOf("size" to size.toDouble(), "error" to "too_big")
    var copied = 0L
    return try {
      into.mkdirs()
      val out = File(into, display)
      resolver.openInputStream(uri)?.use { input ->
        out.outputStream().use { output ->
          val buf = ByteArray(64 * 1024)
          while (true) {
            val n = input.read(buf)
            if (n < 0) break
            copied += n
            if (copied > MAX_BYTES) throw TooBig()
            output.write(buf, 0, n)
          }
        }
      } ?: return base + mapOf("size" to 0.0, "error" to "unreadable")
      base + mapOf("uri" to Uri.fromFile(out).toString(), "size" to copied.toDouble())
    } catch (_: TooBig) {
      into.deleteRecursively()
      base + mapOf("size" to copied.toDouble(), "error" to "too_big")
    } catch (_: Exception) {
      into.deleteRecursively()
      base + mapOf("size" to 0.0, "error" to "unreadable")
    }
  }

  private class TooBig : Exception()
}
