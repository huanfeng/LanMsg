@file:OptIn(kotlinx.coroutines.ExperimentalCoroutinesApi::class)

package name.apk.lanmsg.ui.onboarding

import name.apk.lanmsg.R
import name.apk.lanmsg.ui.common.errorTextRes
import kotlinx.coroutines.test.advanceUntilIdle
import kotlinx.coroutines.test.runTest
import name.apk.lanmsg.core.ApiException
import name.apk.lanmsg.testutil.FakeCoreApi
import name.apk.lanmsg.testutil.MainDispatcherRule
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Rule
import org.junit.Test
import uniffi.lanmsg_api.ErrorCode
import uniffi.lanmsg_api.InitMode
import uniffi.lanmsg_api.Request
import uniffi.lanmsg_api.Response

class OnboardingViewModelTest {
    @get:Rule val main = MainDispatcherRule()

    @Test fun first_device_sends_init_first_and_goes_home() = runTest {
        val fake = FakeCoreApi().apply { handler = { Response.Ok } }
        val vm = OnboardingViewModel(fake, "Pixel 8")
        vm.next(); vm.setDisplayName("小明"); vm.next(); vm.chooseFirstDevice(); advanceUntilIdle()
        assertEquals(Request.Init("小明", "Pixel 8", InitMode.FIRST_DEVICE), fake.requests.last())
        assertEquals(OnboardingViewModel.Destination.HOME, vm.state.value.done)
    }

    @Test fun blank_name_blocks_next() = runTest {
        val vm = OnboardingViewModel(FakeCoreApi(), "Pixel 8")
        vm.next()
        vm.setDisplayName("   ")
        vm.next()
        assertEquals(OnboardingViewModel.Step.NAME, vm.state.value.step)
        assertNotNull(vm.state.value.error)
    }

    @Test fun join_existing_goes_to_pair_join() = runTest {
        val fake = FakeCoreApi().apply { handler = { Response.Ok } }
        val vm = OnboardingViewModel(fake, "Pixel 8")
        vm.next(); vm.setDisplayName("小明"); vm.next(); vm.chooseJoinExisting(); advanceUntilIdle()
        assertEquals(Request.Init("小明", "Pixel 8", InitMode.JOIN_EXISTING), fake.requests.last())
        assertEquals(OnboardingViewModel.Destination.PAIR_JOIN, vm.state.value.done)
    }

    @Test fun init_failure_shows_error_and_stays() = runTest {
        val fake = FakeCoreApi().apply { handler = { throw ApiException(ErrorCode.IO, "disk full") } }
        val vm = OnboardingViewModel(fake, "Pixel 8")
        vm.next(); vm.setDisplayName("小明"); vm.next(); vm.chooseFirstDevice(); advanceUntilIdle()
        assertEquals(errorTextRes(ErrorCode.IO), vm.state.value.error)
        assertNull(vm.state.value.done)
        assertEquals(false, vm.state.value.busy)
    }

    @Test fun unexpected_exception_in_on_initialized_resets_busy() = runTest {
        val fake = FakeCoreApi().apply { handler = { Response.Ok } }
        val vm = OnboardingViewModel(fake, "Pixel 8", onInitialized = { throw IllegalStateException("boom") })
        vm.next(); vm.setDisplayName("小明"); vm.next(); vm.chooseFirstDevice(); advanceUntilIdle()
        assertEquals(false, vm.state.value.busy)
        assertEquals(R.string.err_internal, vm.state.value.error)
        assertNull(vm.state.value.done)
    }
}
