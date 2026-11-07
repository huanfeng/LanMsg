package name.apk.lanmsg.core

import android.util.Log
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineDispatcher
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.CoroutineStart
import kotlinx.coroutines.async
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.Job
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.NonCancellable
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.ensureActive
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.SharedFlow
import kotlinx.coroutines.flow.SharingStarted
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.buffer
import kotlinx.coroutines.flow.combine
import kotlinx.coroutines.flow.emptyFlow
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.flow.flatMapLatest
import kotlinx.coroutines.flow.onStart
import kotlinx.coroutines.flow.shareIn
import kotlinx.coroutines.launch
import kotlinx.coroutines.plus
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withContext
import kotlinx.coroutines.withTimeout
import kotlinx.coroutines.withTimeoutOrNull
import uniffi.lanmsg_api.BackgroundMode
import uniffi.lanmsg_api.ErrorCode
import uniffi.lanmsg_api.Event
import uniffi.lanmsg_api.Request
import uniffi.lanmsg_api.Response
import java.util.concurrent.atomic.AtomicBoolean

/**
 * core 实例的唯一持有者：按 owner 集合管理生命周期，所有 owner 都放手后才 shutdown。
 * 同一时刻最多一个活动实例；新实例一定在旧实例完全关闭之后才创建（两个实例会抢同一个数据库与端口）。
 *
 * @param factory 创建 core（阻塞、读写磁盘），总在 [io] 上调用。
 * @param downloadDir 期望的下载目录；每个新实例发布前会据此调用 [ensureDownloadDir]，返回 null 表示不设置。
 */
@OptIn(ExperimentalCoroutinesApi::class)
class CoreHolder(
    private val factory: () -> CoreApi,
    private val scope: CoroutineScope,
    private val io: CoroutineDispatcher,
    private val downloadDir: () -> String? = { null },
    /** 关闭旧实例最多等多久，超时只记日志放行。 */
    private val shutdownTimeoutMillis: Long = 10_000,
    /** 创建新实例前，对上次超时未结束的 shutdown 再等多久，仍未结束只记日志放行。 */
    private val pendingShutdownWaitMillis: Long = 2_000,
) {
    /** 保护 [ownerSet] 与 [instance] 的写入；只做内存操作，持锁期间不挂起、不阻塞。 */
    private val lock = Any()

    /** 串行化「创建」与「关闭」，保证旧实例关闭完成前不会创建新实例。 */
    private val lifecycle = Mutex()
    private val ownerSet = mutableSetOf<String>()
    private val instance = MutableStateFlow<CoreApi?>(null)
    private val _owners = MutableStateFlow<Set<String>>(emptySet())
    private val _running = MutableStateFlow(false)
    private val _mode = MutableStateFlow<BackgroundMode?>(null)
    private val hasStartedBefore = AtomicBoolean(false)

    /** 超时后仍在运行的 shutdown 任务；只在持有 [lifecycle] 时读写。 */
    private var pendingShutdown: Job? = null

    /** 当前持有者（只读快照）。 */
    val owners: StateFlow<Set<String>> = _owners.asStateFlow()

    val running: StateFlow<Boolean> = _running.asStateFlow()

    /** 后台档位；null = 尚未得知（core 未初始化或还没读到），此时不启动服务。 */
    val backgroundMode: StateFlow<BackgroundMode?> = _mode.asStateFlow()

    /**
     * 给 ViewModel 用的代理：请求发往当前实例。代理自己绝不创建 core、也不登记 owner：
     * 没有实例时，只有 "ui" 已登记（MainActivity 可见、实例正在创建）才等待实例出现；否则抛 [ApiException]。
     * 旧实例被关闭时底层抛出的非 [ApiException] 异常统一转成 [ApiException]。
     * events/lagged 随实例切换（flatMapLatest）；订阅用 Unconfined，实例一发布就接上，尽量不丢事件。
     * 新实例（非首个）出现时额外发一次 lagged，让界面全量重拉。shutdown 对代理是空操作。
     */
    val api: CoreApi = object : CoreApi {
        private val sharing = scope + Dispatchers.Unconfined

        override suspend fun request(req: Request): Response {
            val core = instance.value ?: awaitInstanceForUi()
            try {
                return core.request(req)
            } catch (e: CancellationException) {
                throw e
            } catch (e: ApiException) {
                throw e
            } catch (e: Exception) {
                throw ApiException(ErrorCode.INTERNAL, "Core request failed: ${e.message ?: e.javaClass.simpleName}")
            }
        }

        override val events: SharedFlow<Event> = instance
            .flatMapLatest { it?.events ?: emptyFlow() }
            .buffer(256)
            .shareIn(sharing, SharingStarted.Eagerly)

        override val lagged: SharedFlow<Unit> = instance
            .flatMapLatest { api ->
                api?.lagged?.onStart { if (hasStartedBefore.getAndSet(true)) emit(Unit) } ?: emptyFlow()
            }
            .buffer(1)
            .shareIn(sharing, SharingStarted.Eagerly)

        override fun shutdown() = Unit
    }

    /**
     * 登记 [owner] 并返回实例，首次在 [io] 上创建；并发调用只创建一次，同一 owner 重复调用不重复计数。
     * owner 在创建完成前就被 [release] 掉时，丢弃刚创建的实例并抛 [CancellationException]；
     * 等待中被取消或创建失败时 owner 会被回收。
     *
     * 已知取舍：同名 owner 的两个并发 acquire 中，若其中一个被取消，会回收该 owner 的登记，
     * 另一个随后创建完成时发现 owner 已空而按“已放手”处理（抛 [CancellationException]）。
     * 实际调用方（UI、服务、worker 各自单次 acquire）不会同名并发，故不为此增加按次计数的复杂度。
     */
    suspend fun acquire(owner: String): CoreApi {
        synchronized(lock) {
            ownerSet += owner
            _owners.value = ownerSet.toSet()
        }
        try {
            // 登记之后再检查取消：已取消的调用即使走快路径也要经 catch 回收刚登记的 owner
            currentCoroutineContext().ensureActive()
            instance.value?.let { return it }
            return createAsOwner(owner)
        } catch (e: ReleasedBeforeStart) {
            throw e
        } catch (e: Throwable) {
            // 等锁时被取消、或创建失败：回收 owner，否则它会永久占住 core；
            // 创建完成的同时被取消时实例已发布，owner 归零则一并关闭
            release(owner)
            throw e
        }
    }

    /** owner 在实例创建完成前就被 [release] 掉；此时 owner 已不在集合里，无需回收。 */
    private class ReleasedBeforeStart : CancellationException("Released before the core finished starting")

    private suspend fun createAsOwner(owner: String): CoreApi {
        return lifecycle.withLock {
            instance.value?.let { return it }
            awaitPendingShutdown()
            // 创建 + 发布/关闭整体不可中断：withContext 返回时若外层已取消，结果会被丢弃，
            // 所以必须在返回前就把实例交给 instance（由 acquire 的回收路径负责关闭）或关掉
            val published = withContext(io + NonCancellable) {
                val created = factory().also { applyDownloadDir(it) }
                val publish = synchronized(lock) {
                    (ownerSet.isNotEmpty()).also {
                        if (it) {
                            instance.value = created
                            _running.value = true
                        }
                    }
                }
                if (publish) {
                    created
                } else {
                    shutdownQuietly(created)
                    null
                }
            }
            published ?: throw ReleasedBeforeStart()
        }
    }

    /** 上次 shutdown 超时后仍在跑：新实例与旧实例会抢数据库与端口，创建前再等一小会儿。 */
    private suspend fun awaitPendingShutdown() {
        val job = pendingShutdown ?: return
        if (job.isActive && withTimeoutOrNull(pendingShutdownWaitMillis) { job.join() } == null) {
            Log.w(TAG, "Previous core shutdown still running; creating a new core anyway")
        }
        if (!job.isActive) pendingShutdown = null
    }

    /** 代理用：无实例时，仅当 "ui" 已登记才等待实例出现；"ui" 被放手或从未登记则报错。 */
    private suspend fun awaitInstanceForUi(): CoreApi {
        val (core, _) = combine(instance, _owners) { i, o -> i to o }
            .first { (i, o) -> i != null || UI !in o }
        return core ?: throw ApiException(ErrorCode.INTERNAL, "core not running")
    }

    /** 放手 [owner]；集合归零时丢弃实例并在 [io] 上关闭。关闭完成前，新的 [acquire] 会等待。 */
    fun release(owner: String) {
        synchronized(lock) {
            if (!ownerSet.remove(owner)) return
            _owners.value = ownerSet.toSet()
            if (ownerSet.isNotEmpty()) return
            val old = instance.value ?: return
            instance.value = null
            _running.value = false
            // UNDISPATCHED：在持锁的这一刻就排进 lifecycle 队列，后到的 acquire 一定排在关闭之后
            scope.launch(start = CoroutineStart.UNDISPATCHED) {
                lifecycle.withLock { withContext(io + NonCancellable) { shutdownQuietly(old) } }
            }
        }
    }

    /** 把 core 的 download_dir 设为 [dir]（不同才写）。没有实例或 core 未初始化时容忍失败。 */
    suspend fun ensureDownloadDir(dir: String) {
        applyDownloadDirTo(instance.value ?: return, dir)
    }

    /** 设置页写入后台档位后调用，交给 BackgroundController 重新评估。 */
    fun onBackgroundModeChanged(mode: BackgroundMode) {
        _mode.value = mode
    }

    /** 引导完成（Init 成功）后调用：补设下载目录并读取后台档位。 */
    suspend fun onInitialized() {
        withContext(io) { downloadDir() }?.let { ensureDownloadDir(it) }
        refreshBackgroundMode()
    }

    /** 从 core 读取后台档位；未初始化时保持未知。 */
    suspend fun refreshBackgroundMode() {
        val api = instance.value ?: return
        try {
            if (!api.call<Response.SelfInfo>(Request.GetSelf).value.initialized) return
            _mode.value = api.call<Response.Settings>(Request.GetSettings).value.backgroundMode
        } catch (e: ApiException) {
            Log.w(TAG, "Failed to read background mode: ${e.message}")
        }
    }

    private suspend fun applyDownloadDir(api: CoreApi) {
        downloadDir()?.let { applyDownloadDirTo(api, it) }
    }

    private suspend fun applyDownloadDirTo(api: CoreApi, dir: String) {
        try {
            val s = api.call<Response.Settings>(Request.GetSettings).value
            if (s.downloadDir != dir) {
                s.downloadDir = dir
                api.call<Response.Ok>(Request.SetSettings(s))
            }
        } catch (e: ApiException) {
            // 未初始化时读写设置会失败；引导完成后 onInitialized 会再设一次
            Log.i(TAG, "Download dir not applied: ${e.message}")
        }
    }

    /** 关闭 [core]；最多等 [shutdownTimeoutMillis]，超时只记日志放行，避免后续 acquire 永远等待。 */
    private suspend fun shutdownQuietly(core: CoreApi) {
        // shutdown 是阻塞调用，withTimeout 打断不了它；放到独立任务里，超时后丢下不管
        val job = scope.async(io) { core.shutdown() }
        try {
            withTimeout(shutdownTimeoutMillis) { job.await() }
        } catch (e: Exception) {
            Log.w(TAG, "Core shutdown failed or timed out", e)
            if (job.isActive) pendingShutdown = job
        }
    }

    companion object {
        private const val TAG = "CoreHolder"

        /** MainActivity 可见期间。 */
        const val UI = "ui"

        /** LanMsgService 运行期间。 */
        const val SERVICE = "service"

        /** SyncWorker 执行期间。 */
        const val WORKER = "worker"

        /** BackgroundController 启动时读取后台档位的短暂持有。 */
        const val CONTROLLER = "controller"
    }
}
