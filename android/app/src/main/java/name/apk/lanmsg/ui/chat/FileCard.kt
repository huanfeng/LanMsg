package name.apk.lanmsg.ui.chat

import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.AudioFile
import androidx.compose.material.icons.outlined.Close
import androidx.compose.material.icons.outlined.Description
import androidx.compose.material.icons.outlined.Download
import androidx.compose.material.icons.outlined.FolderZip
import androidx.compose.material.icons.outlined.Image
import androidx.compose.material.icons.automirrored.outlined.InsertDriveFile
import androidx.compose.material.icons.outlined.Videocam
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import name.apk.lanmsg.R
import name.apk.lanmsg.ui.common.fileStatusText
import name.apk.lanmsg.ui.common.Format
import uniffi.lanmsg_api.BlobState
import uniffi.lanmsg_api.FileItemView

enum class FileKind { IMAGE, VIDEO, AUDIO, DOCUMENT, ARCHIVE, OTHER }

/** 按 mime 粗分类，决定图标与是否进入媒体网格。 */
fun fileKindOf(mime: String): FileKind = when {
    mime.startsWith("image/") -> FileKind.IMAGE
    mime.startsWith("video/") -> FileKind.VIDEO
    mime.startsWith("audio/") -> FileKind.AUDIO
    mime.startsWith("text/") || mime == "application/pdf" || mime.contains("word") || mime.contains("excel") ||
        mime.contains("powerpoint") || mime.contains("officedocument") -> FileKind.DOCUMENT
    mime.contains("zip") || mime.contains("compressed") || mime.contains("tar") || mime.contains("rar") ||
        mime.contains("7z") -> FileKind.ARCHIVE
    else -> FileKind.OTHER
}

fun fileKindIcon(kind: FileKind): ImageVector = when (kind) {
    FileKind.IMAGE -> Icons.Outlined.Image
    FileKind.VIDEO -> Icons.Outlined.Videocam
    FileKind.AUDIO -> Icons.Outlined.AudioFile
    FileKind.DOCUMENT -> Icons.Outlined.Description
    FileKind.ARCHIVE -> Icons.Outlined.FolderZip
    FileKind.OTHER -> Icons.AutoMirrored.Outlined.InsertDriveFile
}

/** 当前 Compose 版本没有 MiddleEllipsis：过长的名字保留开头和结尾（含扩展名）。 */
fun middleEllipsis(name: String, max: Int = 28): String =
    if (name.length <= max) name else name.take(max - 11) + "\u2026" + name.takeLast(10)

/**
 * 文件卡片（规范 §5）。[content] 是名称与大小的文字色，[muted] 用于来源等次要文字；
 * 放在“我的”气泡（signal_fill 底）里时两者都传 onFill 以保证对比度。
 */
@OptIn(ExperimentalFoundationApi::class)
@Composable
fun FileCard(
    item: FileItemView,
    progress: Pair<ULong, ULong>?,
    content: Color,
    muted: Color,
    onDownload: (String) -> Unit,
    onCancel: (String) -> Unit,
    onOpen: (FileItemView) -> Unit,
    onLongClick: () -> Unit = {},
    modifier: Modifier = Modifier,
    /** true 时撑满宽度：文字列占满中间、状态标记贴右（列表页用）；聊天气泡保持默认。 */
    fillWidth: Boolean = false,
    /** 并入大小行的附加文字（如时间），显示为“大小 · 附加”。 */
    sizeSuffix: String? = null,
    /** 标题前的小图标（如置顶图钉）。 */
    titlePrefix: (@Composable () -> Unit)? = null,
) {
    val clickable = item.state == BlobState.REMOTE || item.state == BlobState.LOCAL
    val onClick = {
        when (item.state) {
            BlobState.REMOTE -> onDownload(item.hash)
            BlobState.LOCAL -> onOpen(item)
            else -> Unit
        }
    }
    Row(
        modifier
            .then(if (fillWidth) Modifier.fillMaxWidth() else Modifier.widthIn(min = 200.dp))
            .combinedClickable(enabled = clickable, onClick = onClick, onLongClick = onLongClick)
            .padding(vertical = 4.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Icon(fileKindIcon(fileKindOf(item.mime)), contentDescription = null, tint = content, modifier = Modifier.size(32.dp))
        Spacer(Modifier.width(10.dp))
        Column(Modifier.weight(1f, fill = fillWidth)) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                titlePrefix?.invoke()
                Text(
                    middleEllipsis(item.name),
                    color = content,
                    style = MaterialTheme.typography.bodyMedium,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
            }
            Text(
                Format.size(item.size.toLong()) + (sizeSuffix?.let { " \u00B7 $it" } ?: ""),
                color = muted,
                fontSize = 12.sp,
            )
            fileStatusText(item)?.let { Text(it, color = muted, fontSize = 12.sp, maxLines = 1, overflow = TextOverflow.Ellipsis) }
        }
        Spacer(Modifier.width(10.dp))
        FileStateMark(item, progress, muted, onCancel)
    }
}

@Composable
internal fun FileStateMark(item: FileItemView, progress: Pair<ULong, ULong>?, color: Color, onCancel: (String) -> Unit) {
    when (item.state) {
        BlobState.REMOTE -> Column(horizontalAlignment = Alignment.CenterHorizontally) {
            Icon(Icons.Outlined.Download, contentDescription = null, tint = color, modifier = Modifier.size(20.dp))
            Text(stringResource(R.string.file_not_downloaded), color = color, fontSize = 12.sp)
        }
        BlobState.DOWNLOADING -> Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(4.dp)) {
            val fraction = progress?.takeIf { it.second > 0uL }?.let { (it.first.toDouble() / it.second.toDouble()).toFloat().coerceIn(0f, 1f) }
            if (fraction != null) {
                CircularProgressIndicator(progress = { fraction }, color = color, trackColor = Color.Transparent, strokeWidth = 2.dp, modifier = Modifier.size(24.dp))
                Text("${(fraction * 100).toInt()}%", color = color, fontSize = 12.sp)
            } else {
                CircularProgressIndicator(color = color, strokeWidth = 2.dp, modifier = Modifier.size(24.dp))
            }
            IconButton(onClick = { onCancel(item.hash) }, modifier = Modifier.size(32.dp)) {
                Icon(Icons.Outlined.Close, contentDescription = stringResource(R.string.cancel), tint = color, modifier = Modifier.size(18.dp))
            }
        }
        BlobState.LOCAL -> Unit
        BlobState.SOURCE_OFFLINE -> Text(stringResource(R.string.file_source_offline), color = color, fontSize = 12.sp)
        BlobState.PREPARING -> Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(4.dp)) {
            CircularProgressIndicator(color = color, strokeWidth = 2.dp, modifier = Modifier.size(20.dp))
            Text(stringResource(R.string.file_preparing), color = color, fontSize = 12.sp)
        }
    }
}
