// 网页文案（界面文案只放在这里）。按 navigator.language 选择 zh / en。
(function (root) {
  "use strict";
  var S = {
    zh: {
      title: "LanMsg 网页传输",
      code_prompt: "输入宿主屏幕上的 8 位配对码",
      code_placeholder: "配对码",
      code_submit: "连接",
      code_hint_chars: "配对码不含 0、1、I、O",
      code_wrong: "配对码不正确，请重新输入",
      connecting: "正在连接…",
      waiting: "等待对方允许…",
      denied: "对方拒绝了连接",
      verify_failed: "连接校验失败，请重新扫码",
      rate_limited: "尝试过于频繁，请稍后再试",
      not_lan: "只能在局域网内使用",
      net_error: "网络错误，请检查连接后刷新页面",
      ended: "会话已结束",
      host_default: "宿主",
      status_online: "已连接",
      status_offline: "重新连接中…",
      input_placeholder: "输入消息",
      send: "发送",
      pick_files: "选择文件",
      drop_hint: "松开以上传文件",
      download: "下载",
      downloading: "下载中 {0}%",
      uploading: "上传中 {0}",
      upload_failed: "上传失败：{0}",
      download_failed: "下载失败",
      big_file_confirm: "文件较大，浏览器可能占用大量内存，继续吗？",
      from_host: "宿主",
      from_me: "我"
    },
    en: {
      title: "LanMsg Web Transfer",
      code_prompt: "Enter the 8-character pairing code shown on the host",
      code_placeholder: "Pairing code",
      code_submit: "Connect",
      code_hint_chars: "Pairing codes never contain 0, 1, I or O",
      code_wrong: "Wrong pairing code, please try again",
      connecting: "Connecting…",
      waiting: "Waiting for the host to allow…",
      denied: "The host declined the connection",
      verify_failed: "Verification failed, please scan the QR code again",
      rate_limited: "Too many attempts, please try again later",
      not_lan: "Only available on the local network",
      net_error: "Network error, check the connection and reload",
      ended: "Session ended",
      host_default: "Host",
      status_online: "Connected",
      status_offline: "Reconnecting…",
      input_placeholder: "Type a message",
      send: "Send",
      pick_files: "Choose files",
      drop_hint: "Drop files to upload",
      download: "Download",
      downloading: "Downloading {0}%",
      uploading: "Uploading {0}",
      upload_failed: "Upload failed: {0}",
      download_failed: "Download failed",
      big_file_confirm: "This file is large and may use a lot of browser memory. Continue?",
      from_host: "Host",
      from_me: "Me"
    }
  };
  var lang = /^zh/i.test((typeof navigator !== "undefined" && navigator.language) || "") ? "zh" : "en";
  function t(key) {
    var s = (S[lang] && S[lang][key]) || S.en[key] || key;
    for (var i = 1; i < arguments.length; i++) s = s.replace("{" + (i - 1) + "}", arguments[i]);
    return s;
  }
  var api = { t: t, lang: lang, table: S };
  if (typeof module !== "undefined" && module.exports) module.exports = api;
  else root.LanMsgStrings = api;
})(typeof window !== "undefined" ? window : this);
