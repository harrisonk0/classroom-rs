"""macOS generic-password storage through Security.framework; no shell secrets."""
import ctypes as c
import json

security = c.CDLL("/System/Library/Frameworks/Security.framework/Security")
SERVICE = b"local.classroom-rust.desktop.session.v1"
void = c.c_void_p
uint = c.c_uint32
security.SecKeychainFindGenericPassword.argtypes = [void, uint, void, uint, void, c.POINTER(uint), c.POINTER(void), c.POINTER(void)]
security.SecKeychainFindGenericPassword.restype = c.c_int32
security.SecKeychainAddGenericPassword.argtypes = [void, uint, void, uint, void, uint, void, c.POINTER(void)]
security.SecKeychainAddGenericPassword.restype = c.c_int32
security.SecKeychainItemModifyAttributesAndData.argtypes = [void, void, uint, void]
security.SecKeychainItemModifyAttributesAndData.restype = c.c_int32
security.SecKeychainItemFreeContent.argtypes = [void, void]
security.SecKeychainItemDelete.argtypes = [void]
security.SecKeychainItemDelete.restype = c.c_int32
core = c.CDLL("/System/Library/Frameworks/CoreFoundation.framework/CoreFoundation")
core.CFRelease.argtypes = [void]


def lookup(account):
    account = account.encode()
    length, data, item = uint(), void(), void()
    status = security.SecKeychainFindGenericPassword(
        None, len(SERVICE), SERVICE, len(account), account,
        c.byref(length), c.byref(data), c.byref(item))
    if status == -25300:
        return None, None
    if status:
        raise RuntimeError("Keychain access failed")
    try:
        result = json.loads(c.string_at(data, length.value))
    except Exception:
        if item:
            core.CFRelease(item)
        raise
    finally:
        security.SecKeychainItemFreeContent(None, data)
    return result, item


def load(account):
    value, item = lookup(account)
    if item:
        core.CFRelease(item)
    return value


def save(account, value):
    payload = json.dumps(value, separators=(",", ":")).encode()
    _, item = lookup(account)
    try:
        if item:
            status = security.SecKeychainItemModifyAttributesAndData(item, None, len(payload), payload)
        else:
            account_bytes = account.encode()
            status = security.SecKeychainAddGenericPassword(None, len(SERVICE), SERVICE,
                len(account_bytes), account_bytes, len(payload), payload, None)
        if status:
            raise RuntimeError("Keychain save failed")
    finally:
        if item:
            core.CFRelease(item)


def delete(account):
    _, item = lookup(account)
    if item:
        try:
            if security.SecKeychainItemDelete(item):
                raise RuntimeError("Keychain removal failed")
        finally:
            core.CFRelease(item)
