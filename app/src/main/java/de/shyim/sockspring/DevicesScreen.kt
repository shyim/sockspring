@file:OptIn(androidx.compose.material3.ExperimentalMaterial3Api::class)

package de.shyim.sockspring

import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import android.content.Intent
import android.os.Build
import android.os.PersistableBundle
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import com.google.zxing.BarcodeFormat
import com.google.zxing.EncodeHintType
import com.google.zxing.qrcode.QRCodeWriter
import com.google.zxing.qrcode.decoder.ErrorCorrectionLevel
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.withContext

@Composable
internal fun DevicesScreen(
    model: SettingsModel,
    state: ProxyState,
    modifier: Modifier,
    onSettings: () -> Unit,
    onMessage: (String) -> Unit,
) {
    val config = model.saved
    val colors = MaterialTheme.colorScheme
    // The engine reads its device list on start.
    val editable = !state.running && !state.busy
    var adding by rememberSaveable { mutableStateOf(false) }
    var name by rememberSaveable { mutableStateOf("") }
    var shownKey by rememberSaveable { mutableStateOf<String?>(null) }
    var addresses by remember { mutableStateOf(listOf<String>()) }
    LaunchedEffect(config.listen, config.ipv6Only) {
        while (true) {
            addresses = withContext(Dispatchers.IO) { localAddresses(config) }
            delay(5000)
        }
    }
    val host = config.wgEndpoint.ifBlank { addresses.firstOrNull().orEmpty() }

    if (adding)
        AlertDialog(
            onDismissRequest = { adding = false },
            title = { Text("Add a device") },
            text = {
                OutlinedTextField(
                    value = name,
                    onValueChange = { name = it },
                    label = { Text("Name") },
                    placeholder = { Text("Laptop") },
                    singleLine = true,
                    shape = RoundedCornerShape(16.dp),
                )
            },
            confirmButton = {
                TextButton(
                    onClick = {
                        val peer = model.addPeer(name)
                        adding = false
                        name = ""
                        if (peer == null) onMessage("Couldn't add the device")
                        else shownKey = peer.publicKey
                    }
                ) {
                    Text("Add")
                }
            },
            dismissButton = { TextButton(onClick = { adding = false }) { Text("Cancel") } },
        )
    config.wgPeers
        .firstOrNull { it.publicKey == shownKey }
        ?.let { peer ->
            PeerDialog(
                peer = peer,
                text =
                    if (host.isEmpty()) null
                    else config.clientConfig(peer, model.serverPublicKey, host),
                removable = editable,
                onRemove = {
                    shownKey = null
                    model.removePeer(peer)
                    onMessage("${peer.name} removed")
                },
                onCopied = { onMessage("Configuration copied") },
                onClose = { shownKey = null },
            )
        }

    Column(
        modifier.verticalScroll(rememberScrollState()).padding(24.dp),
        verticalArrangement = Arrangement.spacedBy(20.dp),
    ) {
        Text(
            "Your devices,\none tunnel.",
            style = MaterialTheme.typography.headlineLarge,
            fontWeight = FontWeight.Bold,
        )
        Text(
            "Each device gets its own WireGuard key. Scan its QR code with the WireGuard app.",
            color = colors.onSurfaceVariant,
        )
        if (!config.wg)
            Surface(shape = RoundedCornerShape(20.dp), color = colors.secondaryContainer) {
                Column(Modifier.fillMaxWidth().padding(16.dp)) {
                    Text(
                        "The WireGuard server is turned off.",
                        color = colors.onSecondaryContainer,
                    )
                    TextButton(onClick = onSettings) { Text("Open settings") }
                }
            }
        else if (!editable)
            Surface(shape = RoundedCornerShape(20.dp), color = colors.secondaryContainer) {
                Text(
                    "Stop the proxy to add or remove devices.",
                    Modifier.fillMaxWidth().padding(16.dp),
                    color = colors.onSecondaryContainer,
                )
            }
        if (config.wgPeers.isNotEmpty())
            Column(verticalArrangement = Arrangement.spacedBy(10.dp)) {
                SectionLabel("DEVICES")
                config.wgPeers.forEach { peer ->
                    val status = state.peers.firstOrNull { it.publicKey == peer.publicKey }
                    Surface(
                        onClick = { shownKey = peer.publicKey },
                        shape = RoundedCornerShape(24.dp),
                        color = colors.surfaceContainer,
                    ) {
                        Row(
                            Modifier.fillMaxWidth().padding(20.dp),
                            verticalAlignment = Alignment.CenterVertically,
                            horizontalArrangement = Arrangement.spacedBy(14.dp),
                        ) {
                            Box(
                                Modifier.size(10.dp)
                                    .background(
                                        if (status?.connected == true) colors.primary
                                        else colors.outlineVariant,
                                        RoundedCornerShape(50),
                                    )
                            )
                            Column(Modifier.weight(1f)) {
                                Text(
                                    peer.name,
                                    style = MaterialTheme.typography.titleMedium,
                                    fontWeight = FontWeight.SemiBold,
                                )
                                Text(
                                    when {
                                        !state.running -> config.peerAddress4(peer)
                                        status?.connected == true ->
                                            "Connected · ${bytes(status.received)} sent · ${bytes(status.sent)} received"
                                        else -> "Not connected"
                                    },
                                    style = MaterialTheme.typography.bodyMedium,
                                    color = colors.onSurfaceVariant,
                                )
                            }
                            Glyph(R.drawable.ic_chevron)
                        }
                    }
                }
            }
        Button(
            onClick = { adding = true },
            enabled = editable && config.nextPeerHost() != null,
            modifier = Modifier.fillMaxWidth().heightIn(min = 56.dp),
            shape = RoundedCornerShape(50),
        ) {
            Text("Add device", fontWeight = FontWeight.Bold)
        }
        Text(
            "The configuration contains the device's private key. Share it only with the device it is for.",
            style = MaterialTheme.typography.bodySmall,
            color = colors.onSurfaceVariant,
        )
    }
}

@Composable
private fun PeerDialog(
    peer: WgPeer,
    text: String?,
    removable: Boolean,
    onRemove: () -> Unit,
    onCopied: () -> Unit,
    onClose: () -> Unit,
) {
    val context = LocalContext.current
    var confirmRemove by remember(peer.publicKey) { mutableStateOf(false) }
    AlertDialog(
        onDismissRequest = onClose,
        title = { Text(peer.name) },
        text = {
            Column(
                Modifier.verticalScroll(rememberScrollState()),
                verticalArrangement = Arrangement.spacedBy(12.dp),
                horizontalAlignment = Alignment.CenterHorizontally,
            ) {
                if (text == null)
                    Text(
                        "Connect to Wi-Fi or turn on your hotspot to create the configuration, or set an endpoint host in Settings."
                    )
                else {
                    QrCode(text, Modifier.fillMaxWidth().clip(RoundedCornerShape(16.dp)))
                    Text(
                        "In the WireGuard app, add a tunnel and scan this code. Or copy the configuration into a .conf file.",
                        style = MaterialTheme.typography.bodyMedium,
                    )
                    Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                        TextButton(
                            onClick = {
                                copySensitive(context, text)
                                onCopied()
                            }
                        ) {
                            Text("Copy")
                        }
                        TextButton(
                            onClick = {
                                context.startActivity(
                                    Intent.createChooser(
                                        Intent(Intent.ACTION_SEND).apply {
                                            type = "text/plain"
                                            putExtra(Intent.EXTRA_SUBJECT, "${peer.name}.conf")
                                            putExtra(Intent.EXTRA_TEXT, text)
                                        },
                                        "Share configuration",
                                    )
                                )
                            }
                        ) {
                            Text("Share")
                        }
                    }
                }
                TextButton(
                    enabled = removable,
                    onClick = { if (confirmRemove) onRemove() else confirmRemove = true },
                    colors =
                        ButtonDefaults.textButtonColors(
                            contentColor = MaterialTheme.colorScheme.error
                        ),
                ) {
                    Text(
                        when {
                            !removable -> "Stop the proxy to remove"
                            confirmRemove -> "Tap again to remove"
                            else -> "Remove device"
                        }
                    )
                }
            }
        },
        confirmButton = { TextButton(onClick = onClose) { Text("Close") } },
    )
}

/** Always dark on white, whatever the theme, so cameras can read it. */
@Composable
private fun QrCode(text: String, modifier: Modifier) {
    val matrix =
        remember(text) {
            runCatching {
                    QRCodeWriter()
                        .encode(
                            text,
                            BarcodeFormat.QR_CODE,
                            0,
                            0,
                            mapOf(
                                EncodeHintType.MARGIN to 2,
                                EncodeHintType.ERROR_CORRECTION to ErrorCorrectionLevel.L,
                            ),
                        )
                }
                .getOrNull()
        }
    Canvas(
        modifier.aspectRatio(1f).background(Color.White).semantics {
            contentDescription = "QR code with the WireGuard configuration"
        }
    ) {
        if (matrix == null) return@Canvas
        val cell = size.width / matrix.width
        for (y in 0 until matrix.height) for (x in 0 until matrix.width) if (matrix[x, y])
            // Slight overlap hides hairlines between neighbouring modules.
            drawRect(Color.Black, Offset(x * cell, y * cell), Size(cell + 0.5f, cell + 0.5f))
    }
}

// The configuration holds a private key: keep it out of clipboard previews.
private fun copySensitive(context: Context, text: String) {
    val clip = ClipData.newPlainText("WireGuard configuration", text)
    if (Build.VERSION.SDK_INT >= 24)
        clip.description.extras =
            PersistableBundle().apply { putBoolean("android.content.extra.IS_SENSITIVE", true) }
    context.getSystemService(ClipboardManager::class.java).setPrimaryClip(clip)
}
