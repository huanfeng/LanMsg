package name.apk.lanmsg.core

import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.test.runTest
import name.apk.lanmsg.testutil.FakeCoreApi
import name.apk.lanmsg.testutil.Fixtures
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertThrows
import org.junit.Assert.assertTrue
import org.junit.Test
import uniffi.lanmsg_api.ErrorCode
import uniffi.lanmsg_api.Request
import uniffi.lanmsg_api.Response

// 只用 FakeCoreApi，不加载 liblanmsg_ffi.so
class FfiCoreApiMappingTest {
    @Test fun call_extension_unwraps_expected_type() = runTest {
        val fake = FakeCoreApi().apply { handler = { Response.SelfInfo(value = Fixtures.selfInfo()) } }
        val r: Response.SelfInfo = fake.call(Request.GetSelf)
        assertTrue(r.value.initialized)
    }

    @Test fun call_extension_rejects_wrong_type() {
        val fake = FakeCoreApi().apply { handler = { Response.Ok } }
        val e = assertThrows(ApiException::class.java) { runBlocking { fake.call<Response.SelfInfo>(Request.GetSelf) } }
        assertEquals(ErrorCode.INTERNAL, e.code)
    }

    @Test fun is_not_implemented_matches_placeholder_error() {
        assertTrue(ApiException(ErrorCode.INTERNAL, "未实现：QueryFiles").isNotImplemented())
        assertFalse(ApiException(ErrorCode.INTERNAL, "别的错误").isNotImplemented())
        assertFalse(ApiException(ErrorCode.IO, "未实现：x").isNotImplemented())
    }
}
