@file:OptIn(kotlinx.coroutines.ExperimentalCoroutinesApi::class)

package name.apk.lanmsg.ui.contacts

import name.apk.lanmsg.ui.common.errorTextRes
import androidx.test.ext.junit.runners.AndroidJUnit4
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.test.advanceUntilIdle
import kotlinx.coroutines.test.runTest
import name.apk.lanmsg.core.ApiException
import name.apk.lanmsg.testutil.FakeCoreApi
import name.apk.lanmsg.testutil.Fixtures
import name.apk.lanmsg.testutil.MainDispatcherRule
import org.junit.Assert.assertEquals
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import uniffi.lanmsg_api.ErrorCode
import uniffi.lanmsg_api.Event
import uniffi.lanmsg_api.Request
import uniffi.lanmsg_api.Response

@RunWith(AndroidJUnit4::class)
class ContactDetailViewModelTest {
    @get:Rule val main = MainDispatcherRule()

    private fun fake(autoAccept: Boolean = true) = FakeCoreApi().apply {
        handler = { req ->
            when (req) {
                Request.ListContacts -> Response.Contacts(
                    listOf(Fixtures.contact("u2").apply { this.autoAccept = autoAccept }, Fixtures.contact("u3")),
                )
                is Request.OpenDm -> Response.ConvId("c9")
                is Request.SetContactPolicy -> Response.Ok
                else -> throw ApiException(ErrorCode.INTERNAL, "unexpected $req")
            }
        }
    }

    @Test fun loads_contact_by_user() = runTest {
        val vm = ContactDetailViewModel(fake(), "u2")
        advanceUntilIdle()
        assertEquals("u2", vm.state.value.contact?.userId)
    }

    @Test fun open_dm_sets_open_conv() = runTest {
        val fake = fake()
        val vm = ContactDetailViewModel(fake, "u2")
        advanceUntilIdle()
        vm.openDm()
        advanceUntilIdle()
        assertEquals(Request.OpenDm("u2"), fake.requests.last())
        assertEquals("c9", vm.state.value.openConv)
        vm.openConvHandled()
        assertEquals(null, vm.state.value.openConv)
    }

    @Test fun set_ask_above_sends_policy_with_current_auto_accept() = runTest {
        val fake = fake(autoAccept = true)
        val vm = ContactDetailViewModel(fake, "u2")
        advanceUntilIdle()
        vm.setAskAbove(50uL * 1024uL * 1024uL)
        advanceUntilIdle()
        assertEquals(Request.SetContactPolicy("u2", true, 50uL * 1024uL * 1024uL), fake.requests.single { it is Request.SetContactPolicy })
        assertEquals(50uL * 1024uL * 1024uL, vm.state.value.contact?.askAboveBytes)
    }

    @Test fun set_auto_accept_keeps_current_ask_above() = runTest {
        val fake = fake(autoAccept = false)
        val vm = ContactDetailViewModel(fake, "u2")
        advanceUntilIdle()
        vm.setAskAbove(10uL)
        advanceUntilIdle()
        vm.setAutoAccept(true)
        advanceUntilIdle()
        assertEquals(Request.SetContactPolicy("u2", true, 10uL), fake.requests.last())
    }

    @Test fun policy_failure_rolls_back_and_sets_error() = runTest {
        val fake = fake(autoAccept = false)
        val vm = ContactDetailViewModel(fake, "u2")
        advanceUntilIdle()
        fake.handler = { throw ApiException(ErrorCode.IO, "disk") }
        vm.setAutoAccept(true)
        advanceUntilIdle()
        assertEquals(false, vm.state.value.contact?.autoAccept)
        assertEquals(errorTextRes(ErrorCode.IO), vm.state.value.error)
    }

    @Test fun consecutive_policy_failures_end_with_core_value() = runTest {
        val fake = fake(autoAccept = false)
        val vm = ContactDetailViewModel(fake, "u2")
        advanceUntilIdle()
        val ok = fake.handler
        fake.handler = { req -> if (req is Request.SetContactPolicy) throw ApiException(ErrorCode.IO, "disk") else ok(req) }
        vm.setAutoAccept(true)
        vm.setAskAbove(10uL)
        advanceUntilIdle()
        assertEquals(false, vm.state.value.contact?.autoAccept)
        assertEquals(null, vm.state.value.contact?.askAboveBytes)
        assertEquals(errorTextRes(ErrorCode.IO), vm.state.value.error)
    }

    @Test fun failed_first_policy_is_not_carried_by_second() = runTest {
        val fake = fake(autoAccept = false)
        val vm = ContactDetailViewModel(fake, "u2")
        advanceUntilIdle()
        val ok = fake.handler
        var n = 0
        fake.handler = { req -> if (req is Request.SetContactPolicy && n++ == 0) throw ApiException(ErrorCode.IO, "once") else ok(req) }
        vm.setAutoAccept(true)
        vm.setAskAbove(10uL)
        advanceUntilIdle()
        assertEquals(Request.SetContactPolicy("u2", false, 10uL), fake.requests.last { it is Request.SetContactPolicy })
        assertEquals(errorTextRes(ErrorCode.IO), vm.state.value.error)
    }

    @Test fun stale_list_response_does_not_revert_confirmed_after_write() = runTest {
        val fake = fake(autoAccept = false)
        val vm = ContactDetailViewModel(fake, "u2")
        advanceUntilIdle()
        val gate = CompletableDeferred<Unit>()
        var coreAuto = false
        var held = true
        fake.handler = { req ->
            when (req) {
                Request.ListContacts -> {
                    val snapshot = coreAuto // 响应内容在请求发出时就已确定（旧值）
                    if (held) { held = false; gate.await() }
                    Response.Contacts(listOf(Fixtures.contact("u2").apply { autoAccept = snapshot }))
                }
                is Request.SetContactPolicy -> { coreAuto = req.autoAccept; Response.Ok }
                else -> throw ApiException(ErrorCode.INTERNAL, "unexpected $req")
            }
        }
        vm.refresh() // 在途读取（旧值）
        vm.setAutoAccept(true) // 写入排在读取之后
        advanceUntilIdle()
        gate.complete(Unit)
        advanceUntilIdle()
        assertEquals(true, vm.state.value.contact?.autoAccept)
        vm.setAskAbove(10uL)
        advanceUntilIdle()
        assertEquals(Request.SetContactPolicy("u2", true, 10uL), fake.requests.last { it is Request.SetContactPolicy })
    }

    @Test fun refresh_during_write_is_replayed_after_last_write() = runTest {
        val fake = fake(autoAccept = false)
        val vm = ContactDetailViewModel(fake, "u2")
        advanceUntilIdle()
        var coreAuto = false
        fake.handler = { req ->
            when (req) {
                Request.ListContacts -> Response.Contacts(listOf(Fixtures.contact("u2").apply { autoAccept = coreAuto }))
                is Request.SetContactPolicy -> { coreAuto = req.autoAccept; Response.Ok }
                else -> throw ApiException(ErrorCode.INTERNAL, "unexpected $req")
            }
        }
        val before = fake.requests.count { it == Request.ListContacts }
        vm.setAutoAccept(true)
        vm.refresh() // 排在写入之后、写入进行中到达
        advanceUntilIdle()
        assertEquals(true, vm.state.value.contact?.autoAccept)
        assertEquals(before + 1, fake.requests.count { it == Request.ListContacts })
    }

    @Test fun contacts_changed_reloads() = runTest {
        val fake = fake()
        val vm = ContactDetailViewModel(fake, "u2")
        advanceUntilIdle()
        val before = fake.requests.count { it == Request.ListContacts }
        fake.emit(Event.ContactsChanged)
        advanceUntilIdle()
        assertEquals(before + 1, fake.requests.count { it == Request.ListContacts })
    }
}
