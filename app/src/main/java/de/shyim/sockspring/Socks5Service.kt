package de.shyim.sockspring

import android.app.*
import android.content.*
import android.content.pm.ServiceInfo
import android.os.Build
import android.os.IBinder
import androidx.core.app.NotificationCompat
import androidx.core.app.ServiceCompat
import kotlinx.coroutines.*
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import org.json.JSONObject

object NativeBridge {
    init {
        System.loadLibrary("socks5_rs")
    }

    external fun start(config: String): String

    external fun stop()

    external fun snapshot(): String
}

data class ProxyState(
    val phase: String = "stopped",
    val error: String? = null,
    val active: Long = 0,
    val total: Long = 0,
    val uploaded: Long = 0,
    val downloaded: Long = 0,
    val failures: Long = 0,
    val startedAt: Long = 0,
) {
    val running
        get() = phase == "running"

    val busy
        get() = phase == "starting" || phase == "stopping"
}

object ProxyStatus {
    internal val mutable = MutableStateFlow(ProxyState())
    val state = mutable.asStateFlow()
}

class Socks5Service : Service() {
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
    private val operations = Mutex()
    private var monitor: Job? = null

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        if (intent?.action == ACTION_STOP) {
            scope.launch {
                operations.withLock {
                    stopProxy()
                    stopSelf()
                }
            }
            return START_NOT_STICKY
        }
        // Android requires this immediately, before any native startup work.
        showNotification("Starting your proxy…")
        scope.launch {
            operations.withLock {
                if (ProxyStatus.state.value.running) return@withLock
                ProxyStatus.mutable.value = ProxyState(phase = "starting")
                val config = AppConfig.load(this@Socks5Service)
                val validation =
                    if (!canAccessLocalNetwork(this@Socks5Service))
                        "Local network access is required to connect your devices. Allow it in Android app permissions."
                    else config.errors().values.firstOrNull()
                val failure =
                    validation
                        ?: withContext(Dispatchers.IO) {
                            runCatching { NativeBridge.start(config.nativeJson()) }
                                .getOrElse { it.message ?: "Unable to load the proxy engine" }
                                .ifEmpty { null }
                        }
                if (failure != null) {
                    ProxyStatus.mutable.value = ProxyState(error = failure)
                    stopForeground(STOP_FOREGROUND_REMOVE)
                    stopSelf()
                    return@withLock
                }
                ProxyStatus.mutable.value =
                    ProxyState(phase = "running", startedAt = System.currentTimeMillis())
                showNotification("Listening on port ${config.port}")
                monitor?.cancel()
                monitor = scope.launch {
                    while (isActive) {
                        delay(1000)
                        val snapshot =
                            withContext(Dispatchers.IO) { JSONObject(NativeBridge.snapshot()) }
                        val old = ProxyStatus.state.value
                        if (!snapshot.optBoolean("running")) {
                            ProxyStatus.mutable.value =
                                old.copy(
                                    phase = "stopped",
                                    error =
                                        "The proxy stopped unexpectedly. Tap Start to try again.",
                                )
                            stopForeground(STOP_FOREGROUND_REMOVE)
                            stopSelf()
                            break
                        }
                        ProxyStatus.mutable.value =
                            old.copy(
                                active = snapshot.optLong("active"),
                                total = snapshot.optLong("total"),
                                uploaded = snapshot.optLong("uploaded"),
                                downloaded = snapshot.optLong("downloaded"),
                                failures = snapshot.optLong("failures"),
                            )
                    }
                }
            }
        }
        return START_STICKY
    }

    private suspend fun stopProxy() {
        monitor?.cancel()
        monitor = null
        ProxyStatus.mutable.value = ProxyStatus.state.value.copy(phase = "stopping")
        withContext(Dispatchers.IO) { NativeBridge.stop() }
        ProxyStatus.mutable.value =
            ProxyStatus.state.value.copy(phase = "stopped", active = 0, error = null)
        stopForeground(STOP_FOREGROUND_REMOVE)
    }

    override fun onDestroy() {
        scope.cancel()
        // Stop joins only after cancelling all native sessions. No process exit.
        NativeBridge.stop()
        ProxyStatus.mutable.value = ProxyStatus.state.value.copy(phase = "stopped", active = 0)
        super.onDestroy()
    }

    private fun showNotification(message: String) {
        val manager = getSystemService(NotificationManager::class.java)
        if (Build.VERSION.SDK_INT >= 26)
            manager.createNotificationChannel(
                NotificationChannel("proxy", "Proxy status", NotificationManager.IMPORTANCE_LOW)
            )
        val open =
            PendingIntent.getActivity(
                this,
                0,
                Intent(this, MainActivity::class.java),
                PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
            )
        val stop =
            PendingIntent.getService(
                this,
                1,
                Intent(this, Socks5Service::class.java).setAction(ACTION_STOP),
                PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
            )
        val notification =
            NotificationCompat.Builder(this, "proxy")
                .setSmallIcon(R.drawable.ic_proxy)
                .setContentTitle("Sockspring is active")
                .setContentText(message)
                .setContentIntent(open)
                .setOngoing(true)
                .setSilent(true)
                .addAction(R.drawable.ic_stop, "Stop", stop)
                .build()
        ServiceCompat.startForeground(
            this,
            1,
            notification,
            if (Build.VERSION.SDK_INT >= 34) ServiceInfo.FOREGROUND_SERVICE_TYPE_SPECIAL_USE else 0,
        )
    }

    companion object {
        const val ACTION_START = "de.shyim.sockspring.START"
        const val ACTION_STOP = "de.shyim.sockspring.STOP"

        fun canAccessLocalNetwork(context: Context): Boolean =
            Build.VERSION.SDK_INT < 37 ||
                androidx.core.content.ContextCompat.checkSelfPermission(
                    context,
                    "android.permission.ACCESS_LOCAL_NETWORK",
                ) == android.content.pm.PackageManager.PERMISSION_GRANTED

        fun start(context: Context) {
            try {
                androidx.core.content.ContextCompat.startForegroundService(
                    context,
                    Intent(context, Socks5Service::class.java).setAction(ACTION_START),
                )
            } catch (e: RuntimeException) {
                ProxyStatus.mutable.value =
                    ProxyState(error = e.message ?: "Unable to start the proxy")
            }
        }

        fun stop(context: Context) {
            context.startService(Intent(context, Socks5Service::class.java).setAction(ACTION_STOP))
        }
    }
}

class ServiceReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        if (intent.action == Intent.ACTION_BOOT_COMPLETED && AppConfig.load(context).autoStart)
            Socks5Service.start(context)
    }
}
