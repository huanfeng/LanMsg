package name.apk.lanmsg.ui.chat

import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.advanceUntilIdle
import kotlinx.coroutines.test.runTest
import name.apk.lanmsg.testutil.FakeCoreApi
import name.apk.lanmsg.testutil.Fixtures
import name.apk.lanmsg.testutil.MainDispatcherRule
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Rule
import org.junit.Test
import uniffi.lanmsg_api.ConvKind
import uniffi.lanmsg_api.BlobState
import uniffi.lanmsg_api.FileItemView
import uniffi.lanmsg_api.FileKindFilter
import uniffi.lanmsg_api.FileView
import uniffi.lanmsg_api.Request
import uniffi.lanmsg_api.Response

@OptIn(ExperimentalCoroutinesApi::class)
class ImageViewerViewModelTest {
    @get:Rule val main = MainDispatcherRule()

    private fun item(hash: String) = FileItemView(
        hash = hash, name = "$hash.png", size = 10uL, mime = "image/png",
        width = null, height = null, durationMs = null, thumbPath = null, localPath = "/x/$hash.png",
        state = BlobState.LOCAL, sourceName = "设备一", sourceIsMe = false, holderName = null,
    )

    private fun fileView(hash: String, millis: Long) = FileView(item(hash), "c1", "会话", ConvKind.DM, "m-$hash", millis)

    @Test fun image_older_than_recent_200_messages_is_found_via_file_view() = runTest {
        val core = FakeCoreApi()
        // 最近消息里没有目标图片；文件视图分两页，目标在第二页
        val page1 = (0 until 200).map { fileView("n$it", 10_000L - it) }
        val page2 = listOf(fileView("target", 100L))
        core.handler = { req ->
            when (req) {
                is Request.ListMessages -> Response.Messages((0 until 200).map { Fixtures.textMsg("t$it") })
                is Request.QueryFiles -> if (req.filter.beforeMillis == null) Response.Files(page1) else Response.Files(page2)
                else -> error("unexpected $req")
            }
        }
        val vm = ImageViewerViewModel(core, "c1", "target")
        advanceUntilIdle()
        assertEquals("target", vm.state.value.item?.hash)
        val queries = core.requests.filterIsInstance<Request.QueryFiles>()
        assertEquals(2, queries.size)
        assertEquals(FileKindFilter.IMAGE, queries[0].filter.kind)
        assertEquals("c1", queries[0].filter.conv)
        assertEquals(page1.last().hlcMillis, queries[1].filter.beforeMillis)
    }

    @Test fun missing_image_ends_loading_with_no_item() = runTest {
        val core = FakeCoreApi()
        core.handler = { req ->
            when (req) {
                is Request.ListMessages -> Response.Messages(emptyList())
                is Request.QueryFiles -> Response.Files(emptyList())
                else -> error("unexpected $req")
            }
        }
        val vm = ImageViewerViewModel(core, "c1", "nope")
        advanceUntilIdle()
        assertNull(vm.state.value.item)
        assertEquals(false, vm.state.value.loading)
        assertNotNull(vm.state.value)
    }
}
