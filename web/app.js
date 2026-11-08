// LanMsg 网页传输页面逻辑。
// 约定（Task 14 服务端实现）：
// - 页面到宿主的每个加密 JSON、分块与请求头都带 seq（页面侧唯一递增，重试也换新 seq）。
// - 宿主到页面的每个加密 JSON 与分块都带 seq（会话级，WS 与 HTTP 响应共用一个计数器），页面用滑动窗口拒绝重放。
// - finish 与下载请求在 X-LanMsg-Req 头里带密封的 {seq, fid[, idx]}；init 响应回显 client_key，finish 响应回显 fid。
// - 分块明文为 frameChunk(seq, fid, idx, data)，fid 与 idx 在认证范围内。
(function () {
  "use strict";
  var C = window.LanMsgCrypto;
  var T = window.LanMsgStrings.t;
  var enc = new TextEncoder();
  var dec = new TextDecoder();
  var $ = function (id) { return document.getElementById(id); };

  var session = null;
  var keys = null;
  var seq = 0;
  // 宿主到页面的 seq 为会话级单调计数器（WS 帧与 HTTP 加密响应共用），不随重连重置。
  var hostWindow = C.newSeqWindow(4096);
  var ended = false;
  var ws = null;
  var wsDelay = 1000;
  var seen = {};
  var jobs = [];

  document.documentElement.lang = window.LanMsgStrings.lang;
  document.title = T("title");

  function nextSeq() { seq += 1; return seq; }
  function sleep(ms) { return new Promise(function (r) { setTimeout(r, ms); }); }
  function hex(u8) { return Array.prototype.map.call(u8, function (b) { return (b < 16 ? "0" : "") + b.toString(16); }).join(""); }

  function formatSize(n) {
    if (n < 1024) return n + " B";
    if (n < 1048576) return (n / 1024).toFixed(1) + " KB";
    if (n < 1073741824) return (n / 1048576).toFixed(1) + " MB";
    return (n / 1073741824).toFixed(2) + " GB";
  }

  // ---------- 入口页 ----------
  function showGate(text, withForm) {
    $("chat").hidden = true;
    $("gate").hidden = false;
    $("gate-text").textContent = text;
    $("code-form").hidden = !withForm;
    if (withForm) $("code-input").focus();
  }

  // 结束会话并清除密钥材料。
  function endSession() {
    ended = true;
    if (keys) {
      keys.kPage.fill(0);
      keys.kHost.fill(0);
      keys.kEncP2h.fill(0);
      keys.kEncH2p.fill(0);
      keys.th.fill(0);
    }
    keys = null;
    session = null;
  }

  function fail(key) { endSession(); showGate(T(key), false); }

  function postJson(path, body) {
    return fetch(path, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(body)
    });
  }

  function promptCode(msgKey) {
    showGate(T(msgKey || "code_prompt"), true);
  }

  $("code-submit").textContent = T("code_submit");
  $("code-input").placeholder = T("code_placeholder");
  $("code-input").addEventListener("input", function (e) {
    var v = C.normalizeCode(e.target.value);
    e.target.value = v;
    $("code-hint").textContent = /[01IO]/.test(v) ? T("code_hint_chars") : "";
  });
  $("code-form").addEventListener("submit", function (e) {
    e.preventDefault();
    var code = C.normalizeCode($("code-input").value);
    if (code.length !== 8 || /[01IO]/.test(code)) {
      $("code-hint").textContent = T("code_hint_chars");
      return;
    }
    connect("code", code);
  });

  async function connect(mode, secret) {
    showGate(T("connecting"), false);
    try {
      var hs = C.newHandshake();
      var r = await postJson("/api/web/hello", {
        mode: mode, page_pk: C.b64uEncode(hs.pk), page_nonce: C.b64uEncode(hs.nonce)
      });
      if (r.status === 429) return fail("rate_limited");
      if (r.status === 403) return fail("not_lan");
      if (!r.ok) return fail("net_error");
      var h = await r.json();
      var auth = mode === "key" ? secret : await C.scryptCode(secret, C.b64uDecode(h.scrypt_salt));
      var hostPk = C.b64uDecode(h.host_pk);
      keys = C.derive(mode, auth, hs.sk, hostPk, hs.pk, hostPk,
        hs.nonce, C.b64uDecode(h.host_nonce), h.session);
      hs.sk.fill(0);
      if (!keys) return fail("verify_failed");
      session = h.session;
      r = await postJson("/api/web/confirm", { session: session, page_mac: C.b64uEncode(C.pageMac(keys)) });
      if (r.status === 403) {
        if (mode === "code") return promptCode("code_wrong");
        return fail("verify_failed");
      }
      if (r.status === 429) return fail("rate_limited");
      if (!r.ok) return fail("net_error");
      showGate(T("waiting"), false);
      var hostMac = await waitAllowed();
      if (hostMac === "denied") return fail("denied");
      if (hostMac === null) return fail("net_error");
      if (!C.checkHostMac(keys, hostMac)) return fail("verify_failed");
      openChat(h.host_name);
    } catch (err) {
      fail("net_error");
    }
  }

  // 长轮询直到 allowed（返回 host_mac）/ denied / 连续网络错误（返回 null）。
  async function waitAllowed() {
    var errors = 0;
    while (true) {
      try {
        var r = await fetch("/api/web/wait?session=" + encodeURIComponent(session));
        if (!r.ok) throw new Error("status " + r.status);
        var j = await r.json();
        errors = 0;
        if (j.status === "allowed") return C.b64uDecode(j.host_mac);
        if (j.status === "denied") return "denied";
      } catch (err) {
        errors += 1;
        if (errors >= 5) return null;
        await sleep(2000);
      }
    }
  }

  // ---------- 加密 HTTP ----------
  function sealJson(obj) {
    obj.seq = nextSeq();
    return C.seal(keys.kEncP2h, enc.encode(JSON.stringify(obj)));
  }

  // 解密宿主发来的 JSON 并检查 seq（重放或乱序过旧返回 null）。
  function openJson(u8) {
    if (!keys) return null;
    var pt = C.open(keys.kEncH2p, u8);
    if (!pt) return null;
    var m = JSON.parse(dec.decode(pt));
    return hostWindow.accept(m.seq) ? m : null;
  }

  // 带密封请求头 X-LanMsg-Req = b64u(seal({seq, ...fields}))，每次调用换新 seq。
  function reqHeader(fields) {
    var obj = Object.assign({}, fields);
    return C.b64uEncode(sealJson(obj));
  }

  async function secureFetch(method, path, body, reqFields) {
    var headers = { "X-LanMsg-Session": session };
    if (reqFields) headers["X-LanMsg-Req"] = reqHeader(reqFields);
    return fetch(path, { method: method, headers: headers, body: body || undefined });
  }

  // ---------- 聊天 ----------
  function openChat(hostName) {
    $("gate").hidden = true;
    $("chat").hidden = false;
    $("host-name").textContent = hostName || T("host_default");
    $("pick").textContent = T("pick_files");
    $("send-btn").textContent = T("send");
    $("text-input").placeholder = T("input_placeholder");
    $("drop").textContent = T("drop_hint");
    connectWs();
    setInterval(function () { sendWs({ t: "ping" }); }, 20000);
  }

  function setStatus(on) {
    $("dot").className = on ? "dot on" : "dot";
    $("status").textContent = on ? T("status_online") : T("status_offline");
  }

  function sendWs(obj) {
    if (!ws || ws.readyState !== 1) return false;
    ws.send(sealJson(obj));
    return true;
  }

  function connectWs() {
    if (ended) return;
    var proto = location.protocol === "https:" ? "wss:" : "ws:";
    if (!keys) return;
    // 升级请求带认证参数 req（每次重连换新 seq），宿主校验后才接入。
    var req = C.b64uEncode(sealJson({ op: "ws" }));
    ws = new WebSocket(proto + "//" + location.host + "/api/web/ws?session=" + encodeURIComponent(session) + "&req=" + req);
    ws.binaryType = "arraybuffer";
    ws.onopen = function () {
      wsDelay = 1000;
      setStatus(true);
      jobs.forEach(function (j) { if (j.failed) runJob(j); });
    };
    ws.onmessage = function (e) {
      var m;
      try { m = openJson(new Uint8Array(e.data)); } catch (err) { m = null; }
      if (!m) return;
      handleHost(m);
    };
    ws.onclose = function () {
      if (ended) return;
      setStatus(false);
      setTimeout(connectWs, wsDelay);
      wsDelay = Math.min(wsDelay * 2, 15000);
    };
  }

  function handleHost(m) {
    if (m.t === "welcome") {
      if (m.host_name) $("host-name").textContent = m.host_name;
    } else if (m.t === "msg") {
      addMessage(m);
    } else if (m.t === "bye") {
      var sock = ws;
      endSession();
      if (sock) sock.close();
      showGate(T("ended"), false);
    }
  }

  function addMessage(m) {
    if (seen[m.id]) return;
    seen[m.id] = true;
    var li = document.createElement("li");
    li.className = "msg" + (m.from === "page" ? " me" : "");
    var b = document.createElement("div");
    b.className = "bubble";
    if (m.kind === "file" && m.file) {
      renderFile(b, m.file);
    } else {
      b.textContent = m.text || "";
    }
    li.appendChild(b);
    var list = $("list");
    var atBottom = list.scrollHeight - list.scrollTop - list.clientHeight < 80;
    list.appendChild(li);
    if (atBottom) list.scrollTop = list.scrollHeight;
  }

  function renderFile(bubble, f) {
    var name = document.createElement("div");
    name.className = "file-name";
    name.textContent = f.name;
    var meta = document.createElement("div");
    meta.className = "file-meta";
    meta.textContent = formatSize(f.size);
    var btn = document.createElement("button");
    btn.textContent = T("download");
    var prog = document.createElement("progress");
    prog.max = 1;
    prog.value = 0;
    prog.hidden = true;
    bubble.appendChild(name);
    bubble.appendChild(meta);
    bubble.appendChild(btn);
    bubble.appendChild(prog);
    var busy = false;
    btn.addEventListener("click", async function () {
      if (busy) return;
      if (f.size > 1073741824 && !window.confirm(T("big_file_confirm"))) return;
      busy = true;
      prog.hidden = false;
      try {
        var blob = await downloadFile(f, function (p) {
          prog.value = p;
          meta.textContent = T("downloading", Math.round(p * 100));
        });
        meta.textContent = formatSize(f.size);
        prog.hidden = true;
        saveBlob(blob, f.name);
        if (/^image\/(png|jpeg|gif|webp)$/.test(f.mime)) {
          var img = document.createElement("img");
          img.src = URL.createObjectURL(blob);
          img.alt = f.name;
          bubble.appendChild(img);
        }
      } catch (err) {
        meta.textContent = T("download_failed");
        prog.hidden = true;
      }
      busy = false;
    });
  }

  function saveBlob(blob, name) {
    var a = document.createElement("a");
    a.href = URL.createObjectURL(blob);
    a.download = name;
    a.hidden = true;
    document.body.appendChild(a);
    a.click();
    document.body.removeChild(a);
  }

  async function fetchChunk(fid, idx) {
    var lastErr = null;
    for (var attempt = 0; attempt < 3; attempt++) {
      try {
        var r = await secureFetch("GET", "/api/web/download/" + fid + "/" + idx, null, { op: "download", fid: fid, idx: idx });
        if (!r.ok) throw new Error("status " + r.status);
        var pt = keys && C.open(keys.kEncH2p, new Uint8Array(await r.arrayBuffer()));
        var p = pt && C.parseChunk(pt);
        if (!p || p.fid !== fid || p.idx !== idx || !hostWindow.accept(p.seq)) throw new Error("bad chunk");
        return p.data;
      } catch (err) {
        lastErr = err;
        await sleep(500 * (attempt + 1));
      }
    }
    throw lastErr;
  }

  async function downloadFile(f, onProgress) {
    var parts = new Array(f.chunks);
    var next = 0, done = 0;
    async function worker() {
      while (true) {
        var i = next++;
        if (i >= f.chunks) return;
        parts[i] = await fetchChunk(f.fid, i);
        done += 1;
        onProgress(done / f.chunks);
      }
    }
    await Promise.all([worker(), worker(), worker()]);
    return new Blob(parts, { type: f.mime || "application/octet-stream" });
  }

  // ---------- 上传 ----------
  function enqueueFiles(list) {
    Array.prototype.forEach.call(list, function (file) {
      var li = document.createElement("li");
      $("jobs").appendChild(li);
      var job = { file: file, key: hex(C.randomBytes(16)), li: li, failed: false, running: false };
      jobs.push(job);
      runJob(job);
    });
  }

  function jobLabel(job, text) { job.li.textContent = job.file.name + " - " + text; }

  async function putChunk(job, fid, idx, cs) {
    var data = new Uint8Array(await job.file.slice(idx * cs, (idx + 1) * cs).arrayBuffer());
    var lastErr = null;
    for (var attempt = 0; attempt < 3; attempt++) {
      try {
        // 每次尝试都用新 seq 重新封包，否则重试会被宿主当作重放。
        var body = C.seal(keys.kEncP2h, C.frameChunk(nextSeq(), fid, idx, data));
        var r = await secureFetch("PUT", "/api/web/upload/" + fid + "/" + idx, body);
        if (r.status === 204) return;
        throw new Error("status " + r.status);
      } catch (err) {
        lastErr = err;
        await sleep(500 * (attempt + 1));
      }
    }
    throw lastErr;
  }

  async function runJob(job) {
    if (job.running) return;
    job.running = true;
    job.failed = false;
    try {
      var file = job.file;
      var r = await secureFetch("POST", "/api/web/upload/init", sealJson({
        client_key: job.key, name: file.name, size: file.size,
        mime: file.type || "application/octet-stream"
      }));
      if (r.status === 507) throw new Error(T("no_space"));
      if (r.status === 413) throw new Error(T("upload_too_large"));
      if (!r.ok) throw new Error("init " + r.status);
      var init = openJson(new Uint8Array(await r.arrayBuffer()));
      if (!init || init.client_key !== job.key) throw new Error("bad init");
      var cs = init.chunk_size;
      var total = Math.ceil(file.size / cs);
      var have = {};
      init.have.forEach(function (i) { have[i] = true; });
      for (var round = 0; round < 2; round++) {
        var todo = [];
        for (var i = 0; i < total; i++) if (!have[i]) todo.push(i);
        var finished = total - todo.length, pos = 0;
        jobLabel(job, T("uploading", Math.round(total ? finished * 100 / total : 100) + "%"));
        var worker = async function () {
          while (pos < todo.length) {
            var idx = todo[pos++];
            await putChunk(job, init.fid, idx, cs);
            have[idx] = true;
            finished += 1;
            jobLabel(job, T("uploading", Math.round(finished * 100 / total) + "%"));
          }
        };
        await Promise.all([worker(), worker(), worker()]);
        r = await secureFetch("POST", "/api/web/upload/" + init.fid + "/finish", null, { op: "finish", fid: init.fid });
        if (r.ok) {
          var done = openJson(new Uint8Array(await r.arrayBuffer()));
          if (!done || done.fid !== init.fid) throw new Error("bad finish");
          job.li.remove();
          jobs.splice(jobs.indexOf(job), 1);
          job.running = false;
          return;
        }
        if (r.status !== 409) throw new Error("finish " + r.status);
        var miss = openJson(new Uint8Array(await r.arrayBuffer()));
        if (!miss || miss.fid !== init.fid || !miss.missing) throw new Error("bad finish");
        miss.missing.forEach(function (i) { delete have[i]; });
      }
      throw new Error("missing chunks");
    } catch (err) {
      job.failed = true;
      jobLabel(job, T("upload_failed", String(err && err.message || err)));
    }
    job.running = false;
  }

  // ---------- 输入 ----------
  $("send-form").addEventListener("submit", function (e) {
    e.preventDefault();
    var text = $("text-input").value.trim();
    if (!text || enc.encode(text).length > 65536) return;
    if (sendWs({ t: "text", cid: hex(C.randomBytes(8)), text: text })) $("text-input").value = "";
  });
  $("pick").addEventListener("click", function () { $("file-input").click(); });
  $("file-input").addEventListener("change", function (e) {
    enqueueFiles(e.target.files);
    e.target.value = "";
  });
  var dragDepth = 0;
  document.addEventListener("dragenter", function (e) { e.preventDefault(); dragDepth += 1; if (session) $("drop").hidden = false; });
  document.addEventListener("dragover", function (e) { e.preventDefault(); });
  document.addEventListener("dragleave", function () { dragDepth = Math.max(0, dragDepth - 1); if (!dragDepth) $("drop").hidden = true; });
  document.addEventListener("drop", function (e) {
    e.preventDefault();
    dragDepth = 0;
    $("drop").hidden = true;
    if (session && !ended && e.dataTransfer && e.dataTransfer.files.length) enqueueFiles(e.dataTransfer.files);
  });

  // ---------- 启动 ----------
  var hashKey = /(?:^|[#&])k=([A-Za-z0-9_-]+)/.exec(location.hash);
  if (hashKey) {
    try {
      var urlKey = C.b64uDecode(hashKey[1]);
      // 密钥读出后立刻从地址栏与历史记录中移除。
      history.replaceState(null, "", location.pathname + location.search);
      if (urlKey.length === 32) connect("key", urlKey);
      else fail("verify_failed");
    } catch (err) {
      fail("verify_failed");
    }
  } else {
    promptCode();
  }
})();
