package name.apk.lanmsg.ui.common

import name.apk.lanmsg.R
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import uniffi.lanmsg_api.SystemEventView

class SystemEventTextTest {
    private val name = "小李"

    private val cases: List<Triple<SystemEventView, Int, List<Any>>> = listOf(
        Triple(SystemEventView.DeviceAdded(name), R.string.sys_device_added, listOf(name)),
        Triple(SystemEventView.DeviceRemoved(name), R.string.sys_device_removed, listOf(name)),
        Triple(SystemEventView.ContactAdded(name), R.string.sys_contact_added, listOf(name)),
        Triple(SystemEventView.ContactUpdated(name), R.string.sys_contact_updated, listOf(name)),
        Triple(SystemEventView.NicknameUpdated(name), R.string.sys_nickname_updated, listOf(name)),
        Triple(SystemEventView.MessagesDeleted(3u), R.string.sys_messages_deleted, listOf(3)),
        Triple(SystemEventView.Recalled(name, false), R.string.sys_recalled_by_peer, listOf(name)),
        Triple(SystemEventView.Recalled(name, true), R.string.sys_recalled_by_me, emptyList()),
        Triple(SystemEventView.LocalSendPin(name), R.string.sys_localsend_pin, listOf(name)),
        Triple(SystemEventView.LocalSendBusy(name), R.string.sys_localsend_busy, listOf(name)),
    )

    @Test fun every_variant_maps_to_its_resource_with_name_args() {
        for ((event, res, args) in cases) {
            val f = systemEventFormat(event)
            assertEquals("$event", res, f.res)
            assertEquals("$event", args, f.args)
        }
    }

    @Test fun resources_are_distinct() {
        val ids = cases.map { systemEventFormat(it.first).res }
        assertEquals(ids.size, ids.toSet().size)
    }

    @Test fun formats_in_strings_xml_carry_the_name_placeholder() {
        // 单元测试工作目录是 app/，直接读资源文件检查占位符，避免依赖 Robolectric。
        val xml = java.io.File("src/main/res/values/strings.xml").readText()
        val texts = Regex("<string name=\"([a-z_]+)\">([^<]*)</string>").findAll(xml)
            .associate { it.groupValues[1] to it.groupValues[2] }
        for ((event, _, args) in cases) {
            val text = texts.getValue(resName(systemEventFormat(event).res))
            if (args.isEmpty()) assertTrue("$event", !text.contains("%")) else assertTrue("$event", text.contains("%1$"))
        }
    }

    private fun resName(id: Int): String = when (id) {
        R.string.sys_device_added -> "sys_device_added"
        R.string.sys_device_removed -> "sys_device_removed"
        R.string.sys_contact_added -> "sys_contact_added"
        R.string.sys_contact_updated -> "sys_contact_updated"
        R.string.sys_nickname_updated -> "sys_nickname_updated"
        R.string.sys_messages_deleted -> "sys_messages_deleted"
        R.string.sys_recalled_by_peer -> "sys_recalled_by_peer"
        R.string.sys_localsend_pin -> "sys_localsend_pin"
        R.string.sys_localsend_busy -> "sys_localsend_busy"
        else -> "sys_recalled_by_me"
    }
}
