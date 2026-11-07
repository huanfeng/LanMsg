package name.apk.lanmsg.testutil

import uniffi.lanmsg_api.BlobState
import uniffi.lanmsg_api.ContactView
import uniffi.lanmsg_api.ConvKind
import uniffi.lanmsg_api.ConversationSummary
import uniffi.lanmsg_api.DeviceView
import uniffi.lanmsg_api.FileItemView
import uniffi.lanmsg_api.MessageBodyView
import uniffi.lanmsg_api.SystemEventView
import uniffi.lanmsg_api.MessageState
import uniffi.lanmsg_api.MessageView
import uniffi.lanmsg_api.PathClassDto
import uniffi.lanmsg_api.PeerView
import uniffi.lanmsg_api.RequestKind
import uniffi.lanmsg_api.LastMessageKind
import uniffi.lanmsg_api.RequestView
import uniffi.lanmsg_api.SelfInfo
import uniffi.lanmsg_api.StatusDto

object Fixtures {
    fun conv(id: String = "c1", title: String = "", unread: UInt = 0u, last: Long? = 1000, kind: ConvKind = ConvKind.SELF_DEVICES, lastKind: LastMessageKind? = null, lastText: String? = null): ConversationSummary =
        ConversationSummary(
            conv = id, kind = kind, title = title, lastText = lastText, lastKind = lastKind,
            lastHlcMillis = last, unread = unread, peerOnline = true,
        )

    private fun message(id: String, conv: String, mine: Boolean, millis: Long, body: MessageBodyView) = MessageView(
        id = id, conv = conv, authorUser = "u1", authorDevice = if (mine) "d1" else "d2", authorDeviceName = "设备一",
        isMine = mine, hlcMillis = millis, body = body, state = MessageState.SENT,
    )

    fun textMsg(id: String, conv: String = "c1", text: String = "你好", mine: Boolean = false, millis: Long = 1000): MessageView =
        message(id, conv, mine, millis, MessageBodyView.Text(text))

    fun systemMsg(id: String, conv: String = "c1", event: SystemEventView = SystemEventView.MessagesDeleted(1u)): MessageView =
        message(id, conv, false, 1000, MessageBodyView.System(event))

    fun fileMsg(id: String, hash: String, state: BlobState = BlobState.REMOTE): MessageView =
        message(
            id, "c1", false, 1000,
            MessageBodyView.Files(
                listOf(
                    FileItemView(
                        hash = hash, name = "a.bin", size = 10uL, mime = "application/octet-stream",
                        width = null, height = null, durationMs = null, thumbPath = null, localPath = null,
                        state = state, sourceLabel = "设备一",
                    ),
                ),
            ),
        )

    fun selfInfo(initialized: Boolean = true): SelfInfo = SelfInfo(
        initialized = initialized,
        userId = if (initialized) "u1" else null,
        userShort = if (initialized) "U1" else null,
        displayName = if (initialized) "我" else null,
        deviceId = "d1",
        deviceName = "本机",
        status = StatusDto.ONLINE,
    )

    fun peer(deviceId: String = "d2", userId: String = "u2", name: String = "对端", isContact: Boolean = false, isMine: Boolean = false): PeerView =
        PeerView(
            deviceId = deviceId, deviceShort = deviceId.uppercase(), userId = userId, displayName = name,
            deviceName = "对端设备", deviceType = "desktop", addr = "192.168.1.2:9000",
            path = PathClassDto.LAN, isContact = isContact, isMine = isMine,
        )

    fun contact(userId: String = "u2", name: String = "对端", online: Boolean = true): ContactView = ContactView(
        userId = userId, userShort = userId.uppercase(), displayName = name, online = online,
        devices = listOf(DeviceView("d2", "D2", "对端设备", "desktop", online, PathClassDto.LAN, false)),
        autoAccept = false, askAboveBytes = null,
    )

    fun request(id: String = "r1", kind: RequestKind = RequestKind.CONTACT, fromUser: String = "u2", fromName: String = "对端"): RequestView =
        RequestView(
            request = id, kind = kind, fromUser = fromUser, fromName = fromName,
            fromShort = fromUser.uppercase(), preview = "", createdAt = 1000,
        )
}
