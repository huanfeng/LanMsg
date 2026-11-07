package name.apk.lanmsg.ui.convs

import androidx.annotation.StringRes
import name.apk.lanmsg.R
import uniffi.lanmsg_api.ConversationSummary
import uniffi.lanmsg_api.LastMessageKind

/** 会话行第二行的预览：资源 id + 可选格式化参数；[arg] 为空表示不带参数。 */
data class ConversationPreview(@StringRes val res: Int, val arg: String? = null)

/**
 * 按 core 给的结构化类型选预览文案（core 不生成展示文本）。
 * 文本消息返回 null，调用方直接显示 lastText；无消息显示“暂无消息”。
 */
fun conversationPreview(s: ConversationSummary): ConversationPreview? = when (val k = s.lastKind) {
    null -> ConversationPreview(R.string.conv_no_message)
    LastMessageKind.Text ->
        if (s.lastText.isNullOrEmpty()) ConversationPreview(R.string.conv_no_message) else null
    is LastMessageKind.Image -> ConversationPreview(R.string.conv_preview_image)
    is LastMessageKind.Video -> ConversationPreview(R.string.conv_preview_video)
    is LastMessageKind.File ->
        when {
            k.count > 1u -> ConversationPreview(R.string.conv_preview_files, k.count.toString())
            k.name.isEmpty() -> ConversationPreview(R.string.conv_preview_file_plain)
            else -> ConversationPreview(R.string.conv_preview_file, k.name)
        }
    is LastMessageKind.Folder -> ConversationPreview(R.string.conv_preview_folder, k.name)
    LastMessageKind.System -> ConversationPreview(R.string.conv_preview_system)
    LastMessageKind.Recalled -> ConversationPreview(R.string.conv_preview_recalled)
}
