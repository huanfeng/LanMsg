package name.apk.lanmsg.ui.convs

import name.apk.lanmsg.R
import name.apk.lanmsg.testutil.Fixtures
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test
import uniffi.lanmsg_api.LastMessageKind

class ConversationPreviewTest {
    private fun preview(kind: LastMessageKind?, text: String? = null) =
        conversationPreview(Fixtures.conv(lastKind = kind, lastText = text))

    @Test fun no_message_when_kind_absent() =
        assertEquals(ConversationPreview(R.string.conv_no_message), preview(null))

    @Test fun text_uses_last_text() = assertNull(preview(LastMessageKind.Text, "你好"))

    @Test fun image_and_video() {
        assertEquals(ConversationPreview(R.string.conv_preview_image), preview(LastMessageKind.Image(2u)))
        assertEquals(ConversationPreview(R.string.conv_preview_video), preview(LastMessageKind.Video(1u)))
    }

    @Test fun single_file_shows_name_and_many_shows_count() {
        assertEquals(
            ConversationPreview(R.string.conv_preview_file, "a.pdf"),
            preview(LastMessageKind.File("a.pdf", 1u)),
        )
        assertEquals(
            ConversationPreview(R.string.conv_preview_files, "3"),
            preview(LastMessageKind.File("a.pdf", 3u)),
        )
    }

    @Test fun folder_system_recalled() {
        assertEquals(ConversationPreview(R.string.conv_preview_folder, "照片"), preview(LastMessageKind.Folder("照片")))
        assertEquals(ConversationPreview(R.string.conv_preview_system), preview(LastMessageKind.System))
        assertEquals(ConversationPreview(R.string.conv_preview_recalled), preview(LastMessageKind.Recalled))
    }
}
