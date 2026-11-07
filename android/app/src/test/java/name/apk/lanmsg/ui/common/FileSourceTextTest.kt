package name.apk.lanmsg.ui.common

import name.apk.lanmsg.R
import org.junit.Assert.assertEquals
import org.junit.Test
import uniffi.lanmsg_api.BlobState
import uniffi.lanmsg_api.FileItemView

class FileSourceTextTest {
    private fun item(state: BlobState, isMe: Boolean, holder: String?) = FileItemView(
        hash = "h", name = "a.bin", size = 1uL, mime = "application/octet-stream",
        width = null, height = null, durationMs = null, thumbPath = null, localPath = null,
        state = state, sourceName = "张三", sourceIsMe = isMe, holderName = holder,
    )

    @Test fun own_downloaded_file_reads_from_me_downloaded() {
        val f = fileSourceFormat(item(BlobState.LOCAL, true, null))
        assertEquals(R.string.file_source_me_downloaded, f.res)
        assertEquals(emptyList<Any>(), f.args)
    }

    @Test fun remote_file_reads_from_author_stored_at_holder() {
        val f = fileSourceFormat(item(BlobState.REMOTE, false, "张三的笔记本"))
        assertEquals(R.string.file_source_stored, f.res)
        assertEquals(listOf<Any>("张三", "张三的笔记本"), f.args)
    }

    @Test fun downloaded_file_from_other_reads_from_author_downloaded() {
        val f = fileSourceFormat(item(BlobState.LOCAL, false, null))
        assertEquals(R.string.file_source_downloaded, f.res)
        assertEquals(listOf<Any>("张三"), f.args)
    }

    @Test fun file_without_holder_reads_from_author_only() {
        assertEquals(R.string.file_source_from, fileSourceFormat(item(BlobState.SOURCE_OFFLINE, false, null)).res)
        assertEquals(R.string.file_source_me, fileSourceFormat(item(BlobState.SOURCE_OFFLINE, true, null)).res)
    }

    @Test fun own_remote_file_reads_from_me_stored() {
        val f = fileSourceFormat(item(BlobState.REMOTE, true, "手机"))
        assertEquals(R.string.file_source_me_stored, f.res)
        assertEquals(listOf<Any>("手机"), f.args)
    }
}
