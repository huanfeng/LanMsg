package name.apk.lanmsg.ui.nav

import android.net.Uri

/** 路由表，与 docs/ui-architecture.md §2 一致。 */
object Routes {
    const val ONBOARDING = "onboarding"
    const val CONVS = "convs"
    const val FILES = "files?conv={conv}"
    const val CONTACTS = "contacts"
    const val REQUESTS = "requests"
    const val ME = "me"
    const val DEVICES = "devices"
    const val PAIR_HOST = "pair/host"
    const val PAIR_JOIN = "pair/join"
    const val SETTINGS = "settings"
    const val NETWORK = "settings/network"
    const val WEB_HOST = "web/host"
    const val SHARE = "share"
    const val SEARCH = "search"
    const val FORWARD = "forward/{hashes}"
    const val CHAT = "chat/{conv}"
    const val STATION = "station/{conv}"
    const val IMAGE = "image/{conv}/{hash}"
    const val CONTACT = "contact/{user}"
    const val TEXT = "text?path={path}&name={name}&mime={mime}"

    fun chat(conv: String) = "chat/${Uri.encode(conv)}"
    fun station(conv: String) = "station/${Uri.encode(conv)}"
    fun image(conv: String, hash: String) = "image/${Uri.encode(conv)}/${Uri.encode(hash)}"
    fun text(path: String, name: String, mime: String) =
        "text?path=${Uri.encode(path)}&name=${Uri.encode(name)}&mime=${Uri.encode(mime)}"

    fun contact(user: String) = "contact/${Uri.encode(user)}"
    fun files(conv: String? = null) = if (conv == null) "files" else "files?conv=${Uri.encode(conv)}"

    // hash 为 64 位 hex，不含逗号
    fun forward(hashes: List<String>) = "forward/${hashes.joinToString(",")}"
}
