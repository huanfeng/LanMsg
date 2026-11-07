@file:OptIn(kotlinx.coroutines.ExperimentalCoroutinesApi::class)

package name.apk.lanmsg.ui.devices

import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.lifecycle.ViewModelProvider
import androidx.lifecycle.ViewModelStore
import androidx.lifecycle.viewmodel.initializer
import androidx.lifecycle.viewmodel.viewModelFactory
import kotlinx.coroutines.test.advanceTimeBy
import kotlinx.coroutines.test.advanceUntilIdle
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import name.apk.lanmsg.core.ApiException
import name.apk.lanmsg.testutil.FakeCoreApi
import name.apk.lanmsg.testutil.Fixtures
import name.apk.lanmsg.testutil.MainDispatcherRule
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import uniffi.lanmsg_api.ErrorCode
import uniffi.lanmsg_api.Event
import uniffi.lanmsg_api.PairMethod
import uniffi.lanmsg_api.PairSessionInfo
import uniffi.lanmsg_api.PairStateDto
import uniffi.lanmsg_api.PairTarget
import uniffi.lanmsg_api.Request
import uniffi.lanmsg_api.Response

@RunWith(AndroidJUnit4::class)
class PairViewModelTest {
    @get:Rule val main = MainDispatcherRule()

    private var next = 1

    private fun fake() = FakeCoreApi().apply {
        handler = { req ->
            when (req) {
                is Request.PairStart -> Response.PairSession(
                    PairSessionInfo("s${next++}", if (req.method == PairMethod.QR) "lanmsg://pair?x" else null),
                )
                is Request.PairJoin -> Response.PairSession(PairSessionInfo("j${next++}", null))
                is Request.PairConfirm, is Request.PairCancel -> Response.Ok
                Request.ListPeers -> Response.Peers(listOf(Fixtures.peer("d2", userId = "u2", name = "李雷"), Fixtures.peer("d3", userId = ""), Fixtures.peer("d4", userId = "u1", isMine = true)))
                else -> throw ApiException(ErrorCode.INTERNAL, "unexpected $req")
            }
        }
    }

    private fun pairRequests(f: FakeCoreApi) = f.requests.filter { it is Request.PairStart || it is Request.PairJoin || it is Request.PairConfirm || it is Request.PairCancel }

    @Test fun host_start_requests_qr_and_exposes_uri() = runTest {
        val f = fake()
        val vm = PairViewModel(f, PairViewModel.Role.HOST)
        vm.start()
        advanceUntilIdle()
        assertEquals(listOf<Request>(Request.PairStart(PairMethod.QR)), pairRequests(f))
        assertEquals("s1", vm.state.value.session)
        assertEquals("lanmsg://pair?x", vm.state.value.qrUri)
    }

    @Test fun host_switch_to_sas_cancels_old_session() = runTest {
        val f = fake()
        val vm = PairViewModel(f, PairViewModel.Role.HOST)
        vm.start()
        advanceUntilIdle()
        vm.switchToSas()
        advanceUntilIdle()
        assertEquals(
            listOf<Request>(Request.PairStart(PairMethod.QR), Request.PairCancel("s1"), Request.PairStart(PairMethod.SAS)),
            pairRequests(f),
        )
        assertEquals(PairMethod.SAS, vm.state.value.method)
        assertEquals("s2", vm.state.value.session)
        assertNull(vm.state.value.qrUri)
    }

    @Test fun status_for_other_session_is_ignored() = runTest {
        val f = fake()
        val vm = PairViewModel(f, PairViewModel.Role.HOST)
        advanceUntilIdle()
        vm.start()
        advanceUntilIdle()
        f.emit(Event.PairStatus("other", PairStateDto.ShowSas("111111")))
        advanceUntilIdle()
        assertNull(vm.state.value.pairState)
        f.emit(Event.PairStatus("s1", PairStateDto.Verifying))
        advanceUntilIdle()
        assertEquals(PairStateDto.Verifying, vm.state.value.pairState)
    }

    @Test fun status_arriving_before_session_response_is_not_lost() = runTest {
        val f = fake()
        val vm = PairViewModel(f, PairViewModel.Role.HOST)
        advanceUntilIdle()
        val ok = f.handler
        f.handler = { req ->
            val r = ok(req)
            if (req is Request.PairStart) f.emit(Event.PairStatus("s1", PairStateDto.ShowSas("123456")))
            r
        }
        vm.start()
        advanceUntilIdle()
        assertEquals(PairStateDto.ShowSas("123456"), vm.state.value.pairState)
    }

    @Test fun show_sas_then_confirm_sent_once() = runTest {
        val f = fake()
        val vm = PairViewModel(f, PairViewModel.Role.HOST)
        advanceUntilIdle()
        vm.start()
        advanceUntilIdle()
        f.emit(Event.PairStatus("s1", PairStateDto.ShowSas("123456")))
        advanceUntilIdle()
        vm.confirm(true)
        vm.confirm(true)
        advanceUntilIdle()
        assertEquals(1, f.requests.count { it == Request.PairConfirm("s1", true) })
        assertEquals(1, f.requests.count { it is Request.PairConfirm })
    }

    @Test fun join_ignores_non_lanmsg_qr() = runTest {
        val f = fake()
        val vm = PairViewModel(f, PairViewModel.Role.JOIN)
        vm.onQrScanned("https://x")
        advanceUntilIdle()
        assertTrue(pairRequests(f).isEmpty())
    }

    @Test fun join_scans_once_even_if_repeated() = runTest {
        val f = fake()
        val vm = PairViewModel(f, PairViewModel.Role.JOIN)
        vm.onQrScanned("lanmsg://pair?d=1")
        vm.onQrScanned("lanmsg://pair?d=1")
        advanceUntilIdle()
        vm.onQrScanned("lanmsg://pair?d=1")
        advanceUntilIdle()
        assertEquals(listOf<Request>(Request.PairJoin(PairTarget.QrUri("lanmsg://pair?d=1"))), pairRequests(f))
        assertEquals("j1", vm.state.value.session)
    }

    @Test fun join_switch_to_sas_lists_initialized_other_peers_and_pick_joins() = runTest {
        val f = fake()
        val vm = PairViewModel(f, PairViewModel.Role.JOIN)
        vm.switchToSas()
        runCurrent()
        assertEquals(PairViewModel.JoinMode.PICK, vm.state.value.joinMode)
        assertEquals(listOf("d2"), vm.state.value.candidates.map { it.deviceId })
        vm.pick("d2")
        advanceUntilIdle() // 发起加入后轮询已取消，可以跑到空闲
        assertEquals(listOf<Request>(Request.PairJoin(PairTarget.Device("d2"))), pairRequests(f))
    }

    @Test fun pick_mode_polls_every_3s_until_leaving_pick() = runTest {
        val f = fake()
        val vm = PairViewModel(f, PairViewModel.Role.JOIN)
        vm.switchToSas()
        runCurrent()
        assertEquals(1, f.requests.count { it == Request.ListPeers })
        advanceTimeBy(PairViewModel.POLL_MILLIS + 1)
        assertEquals(2, f.requests.count { it == Request.ListPeers })
        advanceTimeBy(PairViewModel.POLL_MILLIS)
        assertEquals(3, f.requests.count { it == Request.ListPeers })
        vm.pick("d2") // 离开 PICK：停止轮询
        runCurrent()
        advanceTimeBy(PairViewModel.POLL_MILLIS * 3)
        assertEquals(3, f.requests.count { it == Request.ListPeers })
    }

    @Test fun manual_refresh_reloads_candidates() = runTest {
        val f = fake()
        val vm = PairViewModel(f, PairViewModel.Role.JOIN)
        vm.switchToSas()
        runCurrent()
        vm.refreshCandidates()
        runCurrent()
        assertEquals(2, f.requests.count { it == Request.ListPeers })
        vm.leave()
    }

    @Test fun host_never_polls_candidates() = runTest {
        val f = fake()
        val vm = PairViewModel(f, PairViewModel.Role.HOST)
        vm.start()
        advanceTimeBy(PairViewModel.POLL_MILLIS * 3)
        assertTrue(f.requests.none { it == Request.ListPeers })
    }

    @Test fun config_change_keeps_session_and_only_clearing_the_viewmodel_cancels() = runTest {
        val f = fake()
        val store = ViewModelStore()
        val factory = viewModelFactory { initializer { PairViewModel(f, PairViewModel.Role.HOST) } }
        // 配置变更：界面销毁重建，ViewModel 由 store 保留，同一个实例
        val vm = ViewModelProvider(store, factory)[PairViewModel::class.java]
        vm.start()
        advanceUntilIdle()
        f.emit(Event.PairStatus("s1", PairStateDto.ShowSas("123456")))
        advanceUntilIdle()
        val again = ViewModelProvider(store, factory)[PairViewModel::class.java]
        advanceUntilIdle()
        assertTrue(vm === again)
        assertEquals("s1", again.state.value.session)
        assertEquals(PairStateDto.ShowSas("123456"), again.state.value.pairState)
        assertTrue(f.requests.none { it is Request.PairCancel })
        // 页面真正离开：导航出栈清理 ViewModel
        store.clear()
        advanceUntilIdle()
        assertEquals(1, f.requests.count { it == Request.PairCancel("s1") })
    }

    @Test fun later_old_session_event_does_not_displace_new_session_early_event() = runTest {
        val f = fake()
        val vm = PairViewModel(f, PairViewModel.Role.HOST)
        advanceUntilIdle()
        val ok = f.handler
        f.handler = { req ->
            val r = ok(req)
            if (req is Request.PairStart) {
                f.emit(Event.PairStatus("s1", PairStateDto.ShowSas("123456")))
                f.emit(Event.PairStatus("old", PairStateDto.Failed("stale")))
            }
            r
        }
        vm.start()
        advanceUntilIdle()
        assertEquals(PairStateDto.ShowSas("123456"), vm.state.value.pairState)
    }

    @Test fun leave_clears_pair_state() = runTest {
        val f = fake()
        val vm = PairViewModel(f, PairViewModel.Role.HOST)
        advanceUntilIdle()
        vm.start()
        advanceUntilIdle()
        f.emit(Event.PairStatus("s1", PairStateDto.ShowSas("123456")))
        advanceUntilIdle()
        vm.leave()
        assertNull(vm.state.value.session)
        assertNull(vm.state.value.pairState)
    }

    @Test fun leave_cancels_unfinished_session() = runTest {
        val f = fake()
        val vm = PairViewModel(f, PairViewModel.Role.HOST)
        vm.start()
        advanceUntilIdle()
        vm.leave()
        vm.leave()
        advanceUntilIdle()
        assertEquals(1, f.requests.count { it == Request.PairCancel("s1") })
    }

    @Test fun leave_after_done_sends_nothing() = runTest {
        val f = fake()
        val vm = PairViewModel(f, PairViewModel.Role.HOST)
        advanceUntilIdle()
        vm.start()
        advanceUntilIdle()
        f.emit(Event.PairStatus("s1", PairStateDto.Done))
        advanceUntilIdle()
        vm.leave()
        advanceUntilIdle()
        assertTrue(f.requests.none { it is Request.PairCancel })
    }

    @Test fun retry_after_failure_starts_a_fresh_host_session() = runTest {
        val f = fake()
        val vm = PairViewModel(f, PairViewModel.Role.HOST)
        advanceUntilIdle()
        vm.start()
        advanceUntilIdle()
        f.emit(Event.PairStatus("s1", PairStateDto.Failed("boom")))
        advanceUntilIdle()
        vm.retry()
        advanceUntilIdle()
        assertEquals("s2", vm.state.value.session)
        assertNull(vm.state.value.pairState)
        assertTrue(f.requests.none { it is Request.PairCancel })
    }
}
