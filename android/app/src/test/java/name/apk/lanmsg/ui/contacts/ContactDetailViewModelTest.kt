@file:OptIn(kotlinx.coroutines.ExperimentalCoroutinesApi::class)

package name.apk.lanmsg.ui.contacts

import androidx.test.ext.junit.runners.AndroidJUnit4
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
        assertEquals("disk", vm.state.value.error)
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
