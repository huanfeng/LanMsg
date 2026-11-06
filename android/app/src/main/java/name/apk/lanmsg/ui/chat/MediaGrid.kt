package name.apk.lanmsg.ui.chat

import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.background
import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import coil.compose.AsyncImage
import name.apk.lanmsg.ui.common.Format
import name.apk.lanmsg.ui.theme.LanMsgColors
import uniffi.lanmsg_api.BlobState
import uniffi.lanmsg_api.FileItemView
import java.io.File

private const val MAX_CELLS = 9

/** 单图最长边 240dp；多图每格固定边长：2 列 96dp、3 列 80dp，格间距 2dp。 */
@OptIn(ExperimentalFoundationApi::class)
@Composable
fun MediaGrid(
    items: List<FileItemView>,
    progress: Map<String, Pair<ULong, ULong>>,
    onClick: (FileItemView) -> Unit,
    onLongClick: () -> Unit,
) {
    if (items.size == 1) {
        val item = items[0]
        val w = item.width?.toFloat() ?: 0f
        val h = item.height?.toFloat() ?: 0f
        val ratio = if (w > 0f && h > 0f) (w / h).coerceIn(0.4f, 2.5f) else 1f
        // 横图宽 240dp，竖图高 240dp
        val boxWidth = if (ratio >= 1f) 240.dp else (240f * ratio).dp
        MediaCell(item, progress[item.hash], 0, Modifier.widthIn(max = boxWidth).aspectRatio(ratio), onClick, onLongClick)
        return
    }
    val shown = items.take(MAX_CELLS)
    val columns = if (shown.size <= 4) 2 else 3
    val cell: Dp = if (columns == 2) 96.dp else 80.dp
    Column(verticalArrangement = Arrangement.spacedBy(2.dp)) {
        for (rowItems in shown.chunked(columns)) {
            Row(horizontalArrangement = Arrangement.spacedBy(2.dp)) {
                for (item in rowItems) {
                    // 超过 9 个时第 9 格叠加 +N（N = 未显示的数量）
                    val extra = if (items.size > MAX_CELLS && item === shown.last()) items.size - MAX_CELLS else 0
                    MediaCell(item, progress[item.hash], extra, Modifier.size(cell), onClick, onLongClick)
                }
            }
        }
    }
}

@OptIn(ExperimentalFoundationApi::class)
@Composable
private fun MediaCell(
    item: FileItemView,
    progress: Pair<ULong, ULong>?,
    extra: Int,
    modifier: Modifier,
    onClick: (FileItemView) -> Unit,
    onLongClick: () -> Unit,
) {
    val c = LanMsgColors.current
    Box(
        modifier.clip(RoundedCornerShape(6.dp)).background(c.canvas).combinedClickable(onClick = { onClick(item) }, onLongClick = onLongClick),
        contentAlignment = Alignment.Center,
    ) {
        val thumb = item.thumbPath
        if (thumb != null) {
            AsyncImage(model = File(thumb), contentDescription = item.name, contentScale = ContentScale.Crop, modifier = Modifier.matchParentSize())
        } else {
            Icon(fileKindIcon(fileKindOf(item.mime)), contentDescription = item.name, tint = c.inkSubtle, modifier = Modifier.size(32.dp))
        }
        if (item.state == BlobState.DOWNLOADING) {
            val fraction = progress?.takeIf { it.second > 0uL }?.let { (it.first.toDouble() / it.second.toDouble()).toFloat().coerceIn(0f, 1f) }
            Box(Modifier.matchParentSize().background(c.surface.copy(alpha = 0.6f)), contentAlignment = Alignment.Center) {
                if (fraction != null) {
                    CircularProgressIndicator(progress = { fraction }, color = c.signal, strokeWidth = 3.dp, modifier = Modifier.size(32.dp))
                } else {
                    CircularProgressIndicator(color = c.signal, strokeWidth = 3.dp, modifier = Modifier.size(32.dp))
                }
            }
        }
        val duration = item.durationMs
        if (fileKindOf(item.mime) == FileKind.VIDEO && duration != null) {
            Text(
                Format.duration(duration.toLong()),
                color = c.ink,
                fontSize = 12.sp,
                modifier = Modifier.align(Alignment.BottomEnd).padding(4.dp).background(c.surface.copy(alpha = 0.85f), RoundedCornerShape(4.dp)).padding(horizontal = 4.dp),
            )
        }
        if (extra > 0) {
            Box(Modifier.matchParentSize().background(c.surface.copy(alpha = 0.8f)), contentAlignment = Alignment.Center) {
                Text("+$extra", color = c.ink, fontSize = 18.sp)
            }
        }
    }
}
