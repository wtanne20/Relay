# Kept in the app's release build, which R8 shrinks (see build.gradle).
#
# WorkManager (used by NoticeWorker) brings Room 2.5, whose own rules keep
# Room databases but not their constructors. R8 dropped the constructor of
# WorkManager's database, Room couldn't create it, and WorkManager's start-up
# (it runs before the app's own code) crashed Relay on every launch.
-keep class * extends androidx.room.RoomDatabase { <init>(); }

# WorkManager creates workers by class name, then calls doWork().
-keep class * extends androidx.work.ListenableWorker {
  public <init>(android.content.Context, androidx.work.WorkerParameters);
}
-keep class expo.modules.relaywatch.NoticeWorker { *; }
