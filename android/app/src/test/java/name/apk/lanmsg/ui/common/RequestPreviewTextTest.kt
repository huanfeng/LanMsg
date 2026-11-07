package name.apk.lanmsg.ui.common

import name.apk.lanmsg.R
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test
import uniffi.lanmsg_api.RequestPreviewView

class RequestPreviewTextTest {
    @Test fun none_has_no_preview() {
        assertNull(requestPreviewFormat(RequestPreviewView.None))
    }

    @Test fun text_is_shown_as_is() {
        val f = requestPreviewFormat(RequestPreviewView.Text("你好"))
        assertEquals(R.string.req_preview_text, f?.res)
        assertEquals(listOf<Any>("你好"), f?.args)
    }

    @Test fun single_file_shows_name_and_size() {
        val f = requestPreviewFormat(RequestPreviewView.Files("a.pdf", 1u, 2048uL))
        assertEquals(R.string.req_preview_file, f?.res)
        assertEquals(listOf<Any>("a.pdf", "2 KB"), f?.args)
    }

    @Test fun several_files_show_count() {
        val f = requestPreviewFormat(RequestPreviewView.Files("a.pdf", 3u, 3145728uL))
        assertEquals(R.string.req_preview_files, f?.res)
        assertEquals(listOf<Any>("a.pdf", 3, "3 MB"), f?.args)
    }

    @Test fun folder_shows_name_and_count() {
        val f = requestPreviewFormat(RequestPreviewView.Folder("docs", 4u))
        assertEquals(R.string.req_preview_folder, f?.res)
        assertEquals(listOf<Any>("docs", 4), f?.args)
    }
}
