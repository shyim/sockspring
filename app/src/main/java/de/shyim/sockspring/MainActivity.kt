@file:OptIn(androidx.compose.material3.ExperimentalMaterial3Api::class)

package de.shyim.sockspring

import android.Manifest
import android.app.Application
import android.content.Intent
import android.content.pm.PackageManager
import android.os.Build
import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.animateColorAsState
import androidx.compose.foundation.background
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.rotate
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalClipboardManager
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.semantics.*
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.text.input.VisualTransformation
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.core.content.ContextCompat
import androidx.lifecycle.AndroidViewModel
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.viewmodel.compose.viewModel
import java.net.Inet4Address
import java.net.Inet6Address
import java.net.NetworkInterface
import java.util.Locale
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

class MainActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge()
        setContent { SocksTheme { SocksApp() } }
    }
}

class SettingsModel(application: Application) : AndroidViewModel(application) {
    var saved by mutableStateOf(AppConfig.load(application))
        private set

    var draft by mutableStateOf(saved)
    var showErrors by mutableStateOf(false)

    fun save(): Boolean {
        showErrors = true
        if (draft.errors().isNotEmpty()) return false
        draft.save(getApplication())
        saved = draft
        showErrors = false
        return true
    }
}

@Composable
private fun SocksTheme(content: @Composable () -> Unit) {
    val dark = isSystemInDarkTheme()
    val colors =
        if (dark)
            darkColorScheme(
                primary = Color(0xFFD0EB97),
                onPrimary = Color(0xFF243409),
                primaryContainer = Color(0xFF34451D),
                onPrimaryContainer = Color(0xFFE0F6B5),
                secondary = Color(0xFFC9BFFF),
                secondaryContainer = Color(0xFF3E375C),
                onSecondaryContainer = Color(0xFFE6DFFF),
                surface = Color(0xFF12150F),
                surfaceContainer = Color(0xFF20241C),
                surfaceContainerHigh = Color(0xFF2A2E25),
                onSurface = Color(0xFFE6E9DC),
                onSurfaceVariant = Color(0xFFBDC4B0),
                outlineVariant = Color(0xFF424A38),
            )
        else
            lightColorScheme(
                primary = Color(0xFF52652C),
                onPrimary = Color.White,
                primaryContainer = Color(0xFFD9F29E),
                onPrimaryContainer = Color(0xFF202C0D),
                secondary = Color(0xFF65558F),
                secondaryContainer = Color(0xFFE9E1FF),
                onSecondaryContainer = Color(0xFF302449),
                surface = Color(0xFFFAF9F6),
                surfaceContainer = Color(0xFFF0F0E9),
                surfaceContainerHigh = Color(0xFFE8E9E0),
                onSurface = Color(0xFF25291F),
                onSurfaceVariant = Color(0xFF5F6656),
                outlineVariant = Color(0xFFD3D7C8),
            )
    MaterialExpressiveTheme(colorScheme = colors, content = content)
}

@Composable
private fun Glyph(id: Int, description: String? = null, modifier: Modifier = Modifier) {
    Icon(painterResource(id), description, modifier.size(24.dp))
}

@Composable
private fun SocksApp(model: SettingsModel = viewModel()) {
    val state by ProxyStatus.state.collectAsStateWithLifecycle()
    val context = LocalContext.current
    var tab by rememberSaveable { mutableIntStateOf(0) }
    val snackbar = remember { SnackbarHostState() }
    val scope = rememberCoroutineScope()
    val permission =
        rememberLauncherForActivityResult(ActivityResultContracts.RequestMultiplePermissions()) {
            if (Socks5Service.canAccessLocalNetwork(context)) Socks5Service.start(context)
            else
                ProxyStatus.mutable.value =
                    ProxyState(
                        error =
                            "Local network access is required to connect your devices. Allow it in Android app permissions."
                    )
        }
    fun start() {
        val missing = buildList {
            if (!Socks5Service.canAccessLocalNetwork(context))
                add("android.permission.ACCESS_LOCAL_NETWORK")
            if (
                Build.VERSION.SDK_INT >= 33 &&
                    ContextCompat.checkSelfPermission(
                        context,
                        Manifest.permission.POST_NOTIFICATIONS,
                    ) != PackageManager.PERMISSION_GRANTED
            )
                add(Manifest.permission.POST_NOTIFICATIONS)
        }
        if (missing.isNotEmpty()) permission.launch(missing.toTypedArray())
        else Socks5Service.start(context)
    }
    Scaffold(
        containerColor = MaterialTheme.colorScheme.surface,
        snackbarHost = { SnackbarHost(snackbar) },
        bottomBar = {
            NavigationBar(containerColor = MaterialTheme.colorScheme.surfaceContainer) {
                NavigationBarItem(
                    selected = tab == 0,
                    onClick = { tab = 0 },
                    icon = { Glyph(R.drawable.ic_proxy) },
                    label = { Text("Overview") },
                )
                NavigationBarItem(
                    selected = tab == 1,
                    onClick = { tab = 1 },
                    icon = { Glyph(R.drawable.ic_settings) },
                    label = { Text("Settings") },
                )
            }
        },
    ) { padding ->
        Box(
            Modifier.fillMaxSize().padding(padding).imePadding(),
            contentAlignment = Alignment.TopCenter,
        ) {
            if (tab == 0)
                Overview(
                    state,
                    model.saved,
                    Modifier.widthIn(max = 640.dp).fillMaxWidth(),
                    onToggle = { if (state.running) Socks5Service.stop(context) else start() },
                    onSettings = { tab = 1 },
                    onCopy = { scope.launch { snackbar.showSnackbar("Proxy address copied") } },
                )
            else
                SettingsScreen(model, state, Modifier.widthIn(max = 640.dp).fillMaxWidth()) {
                    if (model.save()) scope.launch { snackbar.showSnackbar("Settings saved") }
                }
        }
    }
}

@Composable
private fun Overview(
    state: ProxyState,
    config: AppConfig,
    modifier: Modifier,
    onToggle: () -> Unit,
    onSettings: () -> Unit,
    onCopy: () -> Unit,
) {
    val colors = MaterialTheme.colorScheme
    val context = LocalContext.current
    val clipboard = LocalClipboardManager.current
    var addresses by remember { mutableStateOf(listOf<String>()) }
    LaunchedEffect(config.listen, config.ipv6Only) {
        while (true) {
            addresses = withContext(Dispatchers.IO) { localAddresses(config) }
            delay(5000)
        }
    }
    val host = addresses.firstOrNull()
    val endpoint = host?.let { if (':' in it) "[$it]:${config.port}" else "$it:${config.port}" }
    val heroColor by
        animateColorAsState(
            if (state.running) colors.primaryContainer else colors.secondaryContainer,
            label = "status color",
        )
    Column(
        modifier
            .verticalScroll(rememberScrollState())
            .padding(horizontal = 24.dp)
            .padding(top = 24.dp, bottom = 28.dp),
        verticalArrangement = Arrangement.spacedBy(20.dp),
    ) {
        Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
            Column(Modifier.weight(1f)) {
                Text(
                    "Sockspring",
                    style = MaterialTheme.typography.headlineLarge,
                    fontWeight = FontWeight.Bold,
                )
                Text(
                    "A little bridge. More possibilities.",
                    style = MaterialTheme.typography.bodyMedium,
                    color = colors.onSurfaceVariant,
                )
            }
            Surface(shape = RoundedCornerShape(20.dp), color = colors.surfaceContainerHigh) {
                Glyph(R.drawable.ic_proxy, modifier = Modifier.padding(14.dp))
            }
        }
        Surface(shape = RoundedCornerShape(36.dp), color = heroColor) {
            Column(
                Modifier.fillMaxWidth().padding(26.dp),
                verticalArrangement = Arrangement.spacedBy(18.dp),
            ) {
                Row(
                    Modifier.fillMaxWidth(),
                    horizontalArrangement = Arrangement.SpaceBetween,
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    Surface(
                        shape = RoundedCornerShape(50),
                        color = colors.surface.copy(alpha = .65f),
                    ) {
                        Row(
                            Modifier.padding(horizontal = 14.dp, vertical = 8.dp),
                            verticalAlignment = Alignment.CenterVertically,
                            horizontalArrangement = Arrangement.spacedBy(8.dp),
                        ) {
                            Box(
                                Modifier.size(8.dp)
                                    .background(
                                        if (state.running) colors.primary
                                        else colors.onSurfaceVariant,
                                        RoundedCornerShape(50),
                                    )
                            )
                            Text(
                                when (state.phase) {
                                    "running" -> "PROXY IS ON"
                                    "starting" -> "STARTING"
                                    "stopping" -> "STOPPING"
                                    else -> "PROXY IS OFF"
                                },
                                style = MaterialTheme.typography.labelMedium,
                                fontWeight = FontWeight.Bold,
                            )
                        }
                    }
                    Glyph(R.drawable.ic_power, modifier = Modifier.size(28.dp))
                }
                Text(
                    when {
                        state.busy -> "One moment…"
                        state.running -> "You're\nconnected."
                        else -> "Ready when\nyou are."
                    },
                    fontSize = 40.sp,
                    lineHeight = 44.sp,
                    fontWeight = FontWeight.SemiBold,
                    letterSpacing = (-1).sp,
                )
                Text(
                    if (state.running) "Your devices can now connect through this phone."
                    else "Turn your phone into a SOCKS5 proxy in one tap.",
                    style = MaterialTheme.typography.bodyLarge,
                )
                Button(
                    onClick = onToggle,
                    enabled = !state.busy,
                    modifier = Modifier.fillMaxWidth().heightIn(min = 64.dp),
                    shape = RoundedCornerShape(50),
                    colors =
                        ButtonDefaults.buttonColors(
                            containerColor = colors.onPrimaryContainer,
                            contentColor = colors.primaryContainer,
                        ),
                ) {
                    if (state.busy)
                        CircularProgressIndicator(Modifier.size(22.dp), strokeWidth = 2.dp)
                    else Glyph(if (state.running) R.drawable.ic_stop else R.drawable.ic_power)
                    Spacer(Modifier.width(12.dp))
                    Text(
                        if (state.busy) "Please wait"
                        else if (state.running) "Stop proxy" else "Start proxy",
                        style = MaterialTheme.typography.titleMedium,
                        fontWeight = FontWeight.Bold,
                    )
                }
            }
        }
        state.error?.let { message ->
            Surface(
                color = colors.errorContainer,
                shape = RoundedCornerShape(24.dp),
                modifier = Modifier.fillMaxWidth().semantics { liveRegion = LiveRegionMode.Polite },
            ) {
                Column(Modifier.padding(20.dp)) {
                    Text(
                        "Couldn't start the proxy",
                        fontWeight = FontWeight.Bold,
                        color = colors.onErrorContainer,
                    )
                    Text(
                        message,
                        color = colors.onErrorContainer,
                        style = MaterialTheme.typography.bodyMedium,
                    )
                    if (message.startsWith("Local network")) {
                        TextButton(
                            onClick = {
                                context.startActivity(
                                    Intent(
                                        android.provider.Settings
                                            .ACTION_APPLICATION_DETAILS_SETTINGS,
                                        android.net.Uri.parse("package:${context.packageName}"),
                                    )
                                )
                            }
                        ) {
                            Text("App permissions")
                        }
                    } else TextButton(onClick = onSettings) { Text("Review settings") }
                }
            }
        }
        Column(verticalArrangement = Arrangement.spacedBy(10.dp)) {
            SectionLabel("CONNECT YOUR DEVICES")
            Surface(shape = RoundedCornerShape(28.dp), color = colors.surfaceContainer) {
                Column(
                    Modifier.fillMaxWidth().padding(20.dp),
                    verticalArrangement = Arrangement.spacedBy(12.dp),
                ) {
                    Row(verticalAlignment = Alignment.CenterVertically) {
                        Column(Modifier.weight(1f)) {
                            Text(
                                "Proxy address",
                                style = MaterialTheme.typography.labelLarge,
                                color = colors.onSurfaceVariant,
                            )
                            Text(
                                endpoint ?: "No local network",
                                style = MaterialTheme.typography.titleLarge,
                                fontWeight = FontWeight.Bold,
                            )
                        }
                        FilledTonalIconButton(
                            enabled = endpoint != null,
                            onClick = {
                                clipboard.setText(AnnotatedString(endpoint!!))
                                onCopy()
                            },
                        ) {
                            Glyph(R.drawable.ic_copy, "Copy proxy address")
                        }
                    }
                    Text(
                        if (endpoint == null)
                            "Connect to Wi-Fi or turn on your hotspot to see an address."
                        else "On your other device, choose SOCKS5 and enter this address and port.",
                        style = MaterialTheme.typography.bodyMedium,
                        color = colors.onSurfaceVariant,
                    )
                    Row(
                        horizontalArrangement = Arrangement.spacedBy(8.dp),
                        verticalAlignment = Alignment.CenterVertically,
                    ) {
                        SuggestionChip(
                            onClick = onSettings,
                            label = {
                                Text(if (config.auth) "Password protected" else "No authentication")
                            },
                            icon = { Glyph(R.drawable.ic_shield, modifier = Modifier.size(16.dp)) },
                        )
                        TextButton(
                            enabled = endpoint != null,
                            onClick = {
                                context.startActivity(
                                    Intent.createChooser(
                                        Intent(Intent.ACTION_SEND).apply {
                                            type = "text/plain"
                                            putExtra(Intent.EXTRA_TEXT, "SOCKS5 proxy: $endpoint")
                                        },
                                        "Share proxy address",
                                    )
                                )
                            },
                        ) {
                            Text("Share")
                        }
                    }
                    if (addresses.size > 1)
                        Text(
                            "Also available: " +
                                addresses.drop(1).joinToString {
                                    if (':' in it) "[$it]:${config.port}" else "$it:${config.port}"
                                },
                            style = MaterialTheme.typography.bodySmall,
                            color = colors.onSurfaceVariant,
                        )
                }
            }
        }
        Column(verticalArrangement = Arrangement.spacedBy(10.dp)) {
            Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.SpaceBetween) {
                SectionLabel("THIS SESSION")
                Text(
                    if (state.running) elapsed(state.startedAt) else "Not running",
                    style = MaterialTheme.typography.labelMedium,
                    color = colors.onSurfaceVariant,
                )
            }
            Row(horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                Metric(
                    "Uploaded",
                    bytes(state.uploaded),
                    R.drawable.ic_arrow,
                    Modifier.weight(1f),
                    false,
                )
                Metric(
                    "Downloaded",
                    bytes(state.downloaded),
                    R.drawable.ic_arrow,
                    Modifier.weight(1f),
                    true,
                )
            }
            Row(
                Modifier.fillMaxWidth().padding(horizontal = 4.dp),
                horizontalArrangement = Arrangement.SpaceBetween,
            ) {
                Text(
                    "${state.active} active connections",
                    style = MaterialTheme.typography.bodyMedium,
                    color = colors.onSurfaceVariant,
                )
                Text(
                    "${state.total} total",
                    style = MaterialTheme.typography.bodyMedium,
                    color = colors.onSurfaceVariant,
                )
            }
        }
        Surface(
            onClick = onSettings,
            shape = RoundedCornerShape(24.dp),
            color = colors.surfaceContainer,
        ) {
            Row(
                Modifier.fillMaxWidth().padding(20.dp),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(14.dp),
            ) {
                Glyph(R.drawable.ic_settings)
                Column(Modifier.weight(1f)) {
                    Text(
                        "Make it yours",
                        style = MaterialTheme.typography.titleMedium,
                        fontWeight = FontWeight.SemiBold,
                    )
                    Text(
                        "Port, access and performance",
                        style = MaterialTheme.typography.bodyMedium,
                        color = colors.onSurfaceVariant,
                    )
                }
                Glyph(R.drawable.ic_chevron)
            }
        }
    }
}

@Composable
private fun Metric(title: String, value: String, icon: Int, modifier: Modifier, down: Boolean) {
    Surface(
        modifier = modifier,
        shape = RoundedCornerShape(24.dp),
        color = MaterialTheme.colorScheme.surfaceContainer,
    ) {
        Column(Modifier.padding(20.dp), verticalArrangement = Arrangement.spacedBy(10.dp)) {
            Glyph(icon, modifier = Modifier.rotate(if (down) 180f else 0f).size(20.dp))
            Text(
                value,
                style = MaterialTheme.typography.headlineSmall,
                fontWeight = FontWeight.SemiBold,
            )
            Text(
                title,
                style = MaterialTheme.typography.labelLarge,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
    }
}

@Composable
private fun SectionLabel(text: String) {
    Text(
        text,
        style = MaterialTheme.typography.labelMedium,
        fontWeight = FontWeight.Bold,
        letterSpacing = 1.2.sp,
        color = MaterialTheme.colorScheme.onSurfaceVariant,
    )
}

@Composable
private fun SettingsScreen(
    model: SettingsModel,
    state: ProxyState,
    modifier: Modifier,
    onSave: () -> Unit,
) {
    val c = model.draft
    val enabled = !state.running && !state.busy
    val errors = if (model.showErrors) c.errors() else emptyMap()
    var advanced by rememberSaveable { mutableStateOf(false) }
    var reveal by rememberSaveable { mutableStateOf(false) }
    var document by rememberSaveable { mutableStateOf<String?>(null) }
    val context = LocalContext.current
    if (document != null) {
        val file = document!!
        val body =
            remember(file) { context.assets.open(file).bufferedReader().use { it.readText() } }
        AlertDialog(
            onDismissRequest = { document = null },
            title = {
                Text(if (file == "privacy.txt") "Privacy policy" else "Open-source notices")
            },
            text = { Text(body, modifier = Modifier.verticalScroll(rememberScrollState())) },
            confirmButton = { TextButton(onClick = { document = null }) { Text("Close") } },
        )
    }

    Column(
        modifier.verticalScroll(rememberScrollState()).padding(24.dp),
        verticalArrangement = Arrangement.spacedBy(20.dp),
    ) {
        Text(
            "Your proxy,\nyour way.",
            style = MaterialTheme.typography.headlineLarge,
            fontWeight = FontWeight.Bold,
        )
        Text(
            "Simple by default. Flexible when you need it.",
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        if (!enabled)
            Surface(
                shape = RoundedCornerShape(20.dp),
                color = MaterialTheme.colorScheme.secondaryContainer,
            ) {
                Text(
                    "Stop the proxy before changing settings.",
                    Modifier.padding(16.dp),
                    color = MaterialTheme.colorScheme.onSecondaryContainer,
                )
            }
        SettingsGroup("SERVER") {
            Field(
                "Listen address",
                c.listen,
                { model.draft = c.copy(listen = it) },
                enabled,
                errors["listen"],
                "Use :: for all IPv4 and IPv6 interfaces",
            )
            Field(
                "Port",
                c.port,
                { model.draft = c.copy(port = it) },
                enabled,
                errors["port"],
                "Use this port on your other devices",
                number = true,
            )
            Toggle(
                "Start after reboot",
                "Automatically start when your phone restarts",
                c.autoStart,
                enabled,
            ) {
                model.draft = c.copy(autoStart = it)
            }
        }
        SettingsGroup("ACCESS") {
            Toggle(
                "Require authentication",
                "Ask clients for a username and password",
                c.auth,
                enabled,
            ) {
                model.draft = c.copy(auth = it)
            }
            AnimatedVisibility(c.auth) {
                Column(verticalArrangement = Arrangement.spacedBy(14.dp)) {
                    Field(
                        "Username",
                        c.username,
                        { model.draft = c.copy(username = it) },
                        enabled,
                        errors["username"],
                    )
                    OutlinedTextField(
                        value = c.password,
                        onValueChange = { model.draft = c.copy(password = it) },
                        label = { Text("Password") },
                        enabled = enabled,
                        singleLine = true,
                        isError = errors["password"] != null,
                        supportingText = errors["password"]?.let { { Text(it) } },
                        visualTransformation =
                            if (reveal) VisualTransformation.None
                            else PasswordVisualTransformation(),
                        keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Password),
                        trailingIcon = {
                            TextButton(onClick = { reveal = !reveal }) {
                                Text(if (reveal) "Hide" else "Show")
                            }
                        },
                        modifier = Modifier.fillMaxWidth(),
                        shape = RoundedCornerShape(16.dp),
                    )
                }
            }
        }
        SettingsGroup("PERFORMANCE") {
            Toggle(
                "Low-latency TCP",
                "Send small messages immediately. May use more packets.",
                c.noDelay,
                enabled,
            ) {
                model.draft = c.copy(noDelay = it)
            }
            Field(
                "Workers",
                c.workers,
                { model.draft = c.copy(workers = it) },
                enabled,
                errors["workers"],
                "4 is a good starting point; more isn't always faster",
                number = true,
            )
        }
        Surface(
            onClick = { advanced = !advanced },
            color = MaterialTheme.colorScheme.surfaceContainer,
            shape = RoundedCornerShape(24.dp),
        ) {
            Row(
                Modifier.fillMaxWidth().padding(20.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Column(Modifier.weight(1f)) {
                    Text("Advanced networking", fontWeight = FontWeight.SemiBold)
                    Text(
                        "UDP, source addresses and interfaces",
                        style = MaterialTheme.typography.bodySmall,
                    )
                }
                Glyph(R.drawable.ic_chevron, modifier = Modifier.rotate(if (advanced) 90f else 0f))
            }
        }
        AnimatedVisibility(advanced) {
            SettingsGroup("NETWORK OPTIONS") {
                Field(
                    "UDP listen address",
                    c.udpListen,
                    { model.draft = c.copy(udpListen = it) },
                    enabled,
                    errors["udpListen"],
                    "Leave empty to use the incoming connection's address",
                )
                Field(
                    "UDP port or range",
                    c.udpPort,
                    { model.draft = c.copy(udpPort = it) },
                    enabled,
                    errors["udpPort"],
                    "0 chooses an available port automatically",
                )
                Field(
                    "IPv4 source address",
                    c.bindV4,
                    { model.draft = c.copy(bindV4 = it) },
                    enabled,
                    errors["bindV4"],
                )
                Field(
                    "IPv6 source address",
                    c.bindV6,
                    { model.draft = c.copy(bindV6 = it) },
                    enabled,
                    errors["bindV6"],
                )
                Field(
                    "Source interface",
                    c.bindInterface,
                    { model.draft = c.copy(bindInterface = it) },
                    enabled,
                    errors["interface"],
                    "Optional, for example wlan0. Availability depends on Android permissions.",
                )
                Toggle("IPv6 only", "Disable IPv4 on an IPv6 listener", c.ipv6Only, enabled) {
                    model.draft = c.copy(ipv6Only = it)
                }
            }
        }
        if (model.showErrors && errors.isNotEmpty())
            Text(
                "Check the highlighted settings${if(!advanced && errors.keys.any{it in listOf("udpListen","udpPort","bindV4","bindV6","interface")}) " in Advanced networking" else ""}.",
                color = MaterialTheme.colorScheme.error,
            )
        Button(
            onClick = onSave,
            enabled = enabled,
            modifier = Modifier.fillMaxWidth().heightIn(min = 56.dp),
            shape = RoundedCornerShape(50),
        ) {
            Text(
                if (model.draft != model.saved) "Save changes" else "Save settings",
                fontWeight = FontWeight.Bold,
            )
        }
        if (model.draft != model.saved)
            TextButton(
                onClick = {
                    model.draft = model.saved
                    model.showErrors = false
                },
                modifier = Modifier.align(Alignment.CenterHorizontally),
            ) {
                Text("Discard changes")
            }
        TextButton(onClick = { document = "privacy.txt" }) { Text("Privacy policy") }
        TextButton(onClick = { document = "notices.txt" }) { Text("Open-source notices") }
        Text(
            "Sockspring ${BuildConfig.VERSION_NAME}\nTCP · UDP · UDP over TCP",
            modifier = Modifier.align(Alignment.CenterHorizontally),
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
    }
}

@Composable
private fun SettingsGroup(title: String, content: @Composable ColumnScope.() -> Unit) {
    Column(verticalArrangement = Arrangement.spacedBy(10.dp)) {
        SectionLabel(title)
        Surface(
            shape = RoundedCornerShape(28.dp),
            color = MaterialTheme.colorScheme.surfaceContainer,
        ) {
            Column(
                Modifier.fillMaxWidth().padding(20.dp),
                verticalArrangement = Arrangement.spacedBy(16.dp),
                content = content,
            )
        }
    }
}

@Composable
private fun Field(
    label: String,
    value: String,
    onChange: (String) -> Unit,
    enabled: Boolean,
    error: String? = null,
    hint: String? = null,
    number: Boolean = false,
) {
    OutlinedTextField(
        value = value,
        onValueChange = onChange,
        label = { Text(label) },
        singleLine = true,
        enabled = enabled,
        isError = error != null,
        supportingText =
            if (error != null || hint != null) {
                { Text(error ?: hint.orEmpty()) }
            } else null,
        keyboardOptions =
            KeyboardOptions(keyboardType = if (number) KeyboardType.Number else KeyboardType.Ascii),
        modifier = Modifier.fillMaxWidth(),
        shape = RoundedCornerShape(16.dp),
    )
}

@Composable
private fun Toggle(
    title: String,
    description: String,
    value: Boolean,
    enabled: Boolean,
    onChange: (Boolean) -> Unit,
) {
    Row(
        Modifier.fillMaxWidth(),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        Column(Modifier.weight(1f)) {
            Text(
                title,
                style = MaterialTheme.typography.titleSmall,
                fontWeight = FontWeight.SemiBold,
            )
            Text(
                description,
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
        Switch(
            checked = value,
            onCheckedChange = onChange,
            enabled = enabled,
            modifier = Modifier.semantics { contentDescription = title },
        )
    }
}

private fun bytes(n: Long): String {
    return when {
        n < 1024 -> "$n B"
        n < 1024 * 1024 -> String.format(Locale.ROOT, "%.1f KB", n / 1024.0)
        n < 1024L * 1024 * 1024 -> String.format(Locale.ROOT, "%.1f MB", n / (1024.0 * 1024))
        else -> String.format(Locale.ROOT, "%.2f GB", n / (1024.0 * 1024 * 1024))
    }
}

private fun elapsed(start: Long): String {
    val seconds = ((System.currentTimeMillis() - start) / 1000).coerceAtLeast(0)
    return if (seconds < 60) "${seconds}s online"
    else if (seconds < 3600) "${seconds/60}m online"
    else "${seconds/3600}h ${seconds%3600/60}m online"
}

private fun localAddresses(config: AppConfig): List<String> {
    if (config.listen !in listOf("::", "0.0.0.0")) return listOf(config.listen)
    return runCatching {
            NetworkInterface.getNetworkInterfaces()
                .toList()
                .filter { it.isUp && !it.isLoopback }
                .sortedBy { if (it.name.startsWith("wlan") || it.name.startsWith("ap")) 0 else 1 }
                .flatMap { it.inetAddresses.toList() }
                .filter {
                    !it.isLoopbackAddress &&
                        !it.isLinkLocalAddress &&
                        (if (config.ipv6Only) it is Inet6Address
                        else it is Inet4Address && it.isSiteLocalAddress)
                }
                .mapNotNull { it.hostAddress?.substringBefore('%') }
                .distinct()
        }
        .getOrDefault(emptyList())
}
