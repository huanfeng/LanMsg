package name.apk.lanmsg.ui.devices

import android.util.Size
import androidx.camera.core.CameraSelector
import androidx.camera.core.ImageAnalysis
import androidx.camera.core.Preview
import androidx.camera.core.resolutionselector.ResolutionSelector
import androidx.camera.core.resolutionselector.ResolutionStrategy
import androidx.camera.lifecycle.ProcessCameraProvider
import androidx.camera.view.PreviewView
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalLifecycleOwner
import androidx.compose.ui.viewinterop.AndroidView
import androidx.core.content.ContextCompat
import java.util.concurrent.Executors

/** CameraX 预览 + 二维码分析；离开组合时解绑相机。需要已授予 CAMERA 权限。 */
@Composable
fun QrScanner(onResult: (String) -> Unit, modifier: Modifier = Modifier) {
    val context = LocalContext.current
    val lifecycleOwner = LocalLifecycleOwner.current
    val latest = rememberUpdatedState(onResult)
    val previewView = androidx.compose.runtime.remember {
        PreviewView(context).apply { implementationMode = PreviewView.ImplementationMode.COMPATIBLE }
    }

    DisposableEffect(lifecycleOwner) {
        val executor = Executors.newSingleThreadExecutor()
        val future = ProcessCameraProvider.getInstance(context)
        val mainExecutor = ContextCompat.getMainExecutor(context)
        var provider: ProcessCameraProvider? = null
        var disposed = false // 仅主线程访问
        future.addListener({
            if (disposed) return@addListener // 页面已离开：不再绑定，否则无人解绑
            val p = future.get()
            provider = p
            val preview = Preview.Builder().build().also { it.setSurfaceProvider(previewView.surfaceProvider) }
            val analysis = ImageAnalysis.Builder()
                .setResolutionSelector(
                    ResolutionSelector.Builder()
                        .setResolutionStrategy(
                            ResolutionStrategy(Size(1280, 720), ResolutionStrategy.FALLBACK_RULE_CLOSEST_HIGHER_THEN_LOWER),
                        )
                        .build(),
                )
                .setBackpressureStrategy(ImageAnalysis.STRATEGY_KEEP_ONLY_LATEST)
                .build()
                .also { it.setAnalyzer(executor, QrAnalyzer(onResult = { text -> mainExecutor.execute { if (!disposed) latest.value(text) } })) }
            p.unbindAll()
            p.bindToLifecycle(lifecycleOwner, CameraSelector.DEFAULT_BACK_CAMERA, preview, analysis)
        }, mainExecutor)
        onDispose {
            disposed = true
            provider?.unbindAll()
            executor.shutdown()
        }
    }

    AndroidView(factory = { previewView }, modifier = modifier)
}
