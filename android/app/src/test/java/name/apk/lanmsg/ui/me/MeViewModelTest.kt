@file:OptIn(kotlinx.coroutines.ExperimentalCoroutinesApi::class)

package name.apk.lanmsg.ui.me

import androidx.test.ext.junit.runners.AndroidJUnit4
import kotlinx.coroutines.test.advanceUntilIdle
import kotlinx.coroutines.test.runTest
import name.apk.lanmsg.core.ApiException
import name.apk.lanmsg.testutil.FakeCoreApi
import name.apk.lanmsg.testutil.Fixtures
import name.apk.lanmsg.testutil.MainDispatcherRule
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import uniffi.lanmsg_api.ErrorCode
import uniffi.lanmsg_api.Request
import uniffi.lanmsg_api.Response
import uniffi.lanmsg_api.SelfInfo
import uniffi.lanmsg_api.StatusDto

@RunWith(AndroidJUnit4::class)
class MeViewModelTest {
    @get:Rule val main = MainDispatcherRule()

    private var current: SelfInfo = Fixtures.selfInfo()

    private fun fake() = FakeCoreApi().apply {
        handler = { req ->
            when (req) {
                Request.GetSelf -> Response.SelfInfo(current.copy())
                is Request.SetStatus -> { current = current.copy(status = req.status); Response.Ok }
                is Request.SetProfile -> { current = current.copy(displayName = req.displayName); Response.Ok }
                else -> throw ApiException(ErrorCode.INTERNAL, "unexpected $req")
            }
        }
    }

    @Test fun set_status_sends_request_and_reloads_self() = runTest {
        val fake = fake()
        val vm = MeViewModel(fake)
        advanceUntilIdle()
        vm.setStatus(StatusDto.BUSY)
        advanceUntilIdle()
        assertEquals(Request.SetStatus(StatusDto.BUSY), fake.requests.single { it is Request.SetStatus })
        assertEquals(StatusDto.BUSY, vm.state.value.self?.status)
    }

    @Test fun rename_sends_trimmed_name_and_reloads() = runTest {
        val fake = fake()
        val vm = MeViewModel(fake)
        advanceUntilIdle()
        vm.rename("  Ming2 ")
        advanceUntilIdle()
        assertEquals(Request.SetProfile("Ming2"), fake.requests.single { it is Request.SetProfile })
        assertEquals("Ming2", vm.state.value.self?.displayName)
    }

    @Test fun rename_blank_is_ignored() = runTest {
        val fake = fake()
        val vm = MeViewModel(fake)
        advanceUntilIdle()
        vm.rename("   ")
        advanceUntilIdle()
        assertEquals(0, fake.requests.count { it is Request.SetProfile })
    }

    @Test fun rename_failure_sets_error() = runTest {
        val fake = fake()
        val vm = MeViewModel(fake)
        advanceUntilIdle()
        fake.handler = { throw ApiException(ErrorCode.IO, "disk") }
        vm.rename("x")
        advanceUntilIdle()
        assertNotNull(vm.state.value.error)
    }
}
