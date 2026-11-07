package name.apk.lanmsg.ui.common

import androidx.compose.runtime.Composable
import androidx.compose.ui.res.stringResource
import name.apk.lanmsg.R
import uniffi.lanmsg_api.RequestPreviewView

/** 请求内容预览 → 资源 id + 参数；没有可预览内容时为 null（界面只显示请求类型）。 */
fun requestPreviewFormat(p: RequestPreviewView): EventText? = when (p) {
    is RequestPreviewView.None -> null
    is RequestPreviewView.Text -> EventText(R.string.req_preview_text, listOf(p.text))
    is RequestPreviewView.Files ->
        if (p.count <= 1u) EventText(R.string.req_preview_file, listOf(p.firstName, Format.size(p.totalSize.toLong())))
        else EventText(R.string.req_preview_files, listOf(p.firstName, p.count.toInt(), Format.size(p.totalSize.toLong())))
    is RequestPreviewView.Folder -> EventText(R.string.req_preview_folder, listOf(p.name, p.count.toInt()))
}

@Composable
fun requestPreviewText(p: RequestPreviewView): String? {
    val f = requestPreviewFormat(p) ?: return null
    return stringResource(f.res, *f.args.toTypedArray())
}
