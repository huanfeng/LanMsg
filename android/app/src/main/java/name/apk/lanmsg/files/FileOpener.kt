package name.apk.lanmsg.files

import android.content.ActivityNotFoundException
import android.content.Context
import android.content.Intent
import android.widget.Toast
import androidx.core.content.FileProvider
import name.apk.lanmsg.R
import java.io.File

object FileOpener {
    /** 与 AndroidManifest 中 FileProvider 的 authorities 一致。 */
    const val AUTHORITY = "name.apk.lanmsg.files"

    fun open(ctx: Context, localPath: String, mime: String) {
        try {
            val uri = FileProvider.getUriForFile(ctx, AUTHORITY, File(localPath))
            val intent = Intent(Intent.ACTION_VIEW)
                .setDataAndType(uri, mime.ifBlank { "*/*" })
                .addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
                .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
            ctx.startActivity(intent)
        } catch (e: ActivityNotFoundException) {
            toastFailure(ctx, e)
        } catch (e: IllegalArgumentException) {
            // 路径不在 file_paths.xml 声明的范围内
            toastFailure(ctx, e)
        }
    }

    private fun toastFailure(ctx: Context, e: Exception) {
        Toast.makeText(ctx, ctx.getString(R.string.err_generic, e.message ?: e.javaClass.simpleName), Toast.LENGTH_SHORT).show()
    }
}
