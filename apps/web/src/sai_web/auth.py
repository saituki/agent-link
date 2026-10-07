"""アカウント・ログイン・個人のトークン（実装計画 U06）。

- パスワード: 標準ライブラリの `hashlib.scrypt` で保存する（依存を増やさない）。
- 名前・メールアドレス・パスワードの検査。
- ログインのセッション（Cookie `sai_session`）と個人のトークン（`sai_…`）。
- 管理者だけを通す依存関数 `require_admin`。

値そのもの（パスワード・Cookie・トークン）は DB に置かない。パスワードは scrypt の
文字列、Cookie とトークンは SHA-256 だけを `store` が保存する。
"""

from __future__ import annotations

import base64
import hashlib
import hmac
import os
import re
import secrets

from fastapi import HTTPException, Response

from sai_web import registry

# パスワードの長さ（共有契約 C5'・決定事項）
PASSWORD_MIN = 8
PASSWORD_MAX = 128

# 短い名前: 英小文字で始まり、英小文字と数字が続く（2〜16 文字。ハイフンは許さない）。
# プロジェクトのフォルダ名 `<名前>-<プロジェクト>` が一意になるため（共有契約 C4）。
_NAME_RE = re.compile(r"^[a-z][a-z0-9]{1,15}$")
# メールアドレス: ざっくりした形だけ（`@` と `.` があり、空白を含まない）。
_EMAIL_RE = re.compile(r"^[^@\s]+@[^@\s]+\.[^@\s]+$")

# ログインのセッション（Cookie の名前と期限）
SESSION_COOKIE = "sai_session"
SESSION_DAYS = 30
# 個人のトークンの頭（内部の合言葉と区別する・共有契約 C5）
API_TOKEN_PREFIX = "sai_"
# scrypt のパラメータ（共有契約 C5' の指示値）
_SCRYPT_N = 2**14
_SCRYPT_R = 8
_SCRYPT_P = 1
_SCRYPT_MAXMEM = 2**26
_SCRYPT_DKLEN = 64


# --- 値の検査 ---


def check_name(name: str) -> str:
    """短い名前を検査して返す（違えば `ValueError`）。"""
    value = (name or "").strip()
    if not _NAME_RE.match(value):
        raise ValueError(
            "名前は英小文字で始まる 2〜16 文字（英小文字と数字）にしてください"
        )
    return value


def check_email(email: str) -> str:
    """メールアドレスを検査して小文字にして返す（違えば `ValueError`）。"""
    value = (email or "").strip()
    if len(value) > 254 or not _EMAIL_RE.match(value):
        raise ValueError("メールアドレスの形が正しくありません")
    return value.lower()


def check_password(password: str) -> None:
    """パスワードの長さを検査する（違えば `ValueError`）。"""
    if not isinstance(password, str) or not (PASSWORD_MIN <= len(password) <= PASSWORD_MAX):
        raise ValueError(
            f"パスワードは {PASSWORD_MIN} 文字以上 {PASSWORD_MAX} 文字以下にしてください"
        )


# --- パスワードの保存と照合 ---


def _b64(data: bytes) -> str:
    return base64.b64encode(data).decode("ascii")


def _unb64(text: str) -> bytes:
    return base64.b64decode(text.encode("ascii"))


def hash_password(password: str) -> str:
    """パスワードを `scrypt$<n>$<r>$<p>$<salt b64>$<hash b64>` の形にする。"""
    salt = os.urandom(16)
    dk = hashlib.scrypt(
        password.encode("utf-8"),
        salt=salt,
        n=_SCRYPT_N,
        r=_SCRYPT_R,
        p=_SCRYPT_P,
        maxmem=_SCRYPT_MAXMEM,
        dklen=_SCRYPT_DKLEN,
    )
    return f"scrypt${_SCRYPT_N}${_SCRYPT_R}${_SCRYPT_P}${_b64(salt)}${_b64(dk)}"


def verify_password(password: str, stored: str) -> bool:
    """パスワードが保存された scrypt 文字列と合うか（定数時間で比べる）。"""
    if not isinstance(stored, str):
        return False
    parts = stored.split("$")
    if len(parts) != 6 or parts[0] != "scrypt":
        return False
    try:
        n, r, p = int(parts[1]), int(parts[2]), int(parts[3])
        salt = _unb64(parts[4])
        expected = _unb64(parts[5])
        dk = hashlib.scrypt(
            password.encode("utf-8"),
            salt=salt,
            n=n,
            r=r,
            p=p,
            maxmem=_SCRYPT_MAXMEM,
            dklen=len(expected),
        )
    except (ValueError, TypeError):
        return False
    return hmac.compare_digest(dk, expected)


# --- トークン ---


def sha256_hex(value: str) -> str:
    """DB に置くハッシュ（Cookie と個人のトークン）。"""
    return hashlib.sha256(value.encode("utf-8")).hexdigest()


def new_session_token() -> str:
    """ログインの Cookie に入れる値。"""
    return secrets.token_urlsafe(32)


def new_api_token() -> str:
    """個人のトークンの値（`sai_` で始まる）。見せるのは作った 1 回だけ。"""
    return API_TOKEN_PREFIX + secrets.token_urlsafe(32)


# --- Cookie ---


def cookie_secure() -> bool:
    """Cookie に `Secure` を付けるか（`SAI_WEB_COOKIE_SECURE=0` で外す）。

    手元の http での確認用。既定は付ける（1）。
    """
    return (os.getenv("SAI_WEB_COOKIE_SECURE", "1") or "1").strip() != "0"


def set_session_cookie(response: Response, token: str) -> None:
    """ログインの Cookie を付ける（HttpOnly・SameSite=Lax・Path=/）。"""
    response.set_cookie(
        key=SESSION_COOKIE,
        value=token,
        max_age=SESSION_DAYS * 86400,
        httponly=True,
        samesite="lax",
        secure=cookie_secure(),
        path="/",
    )


def clear_session_cookie(response: Response) -> None:
    """ログインの Cookie を消す。"""
    response.delete_cookie(SESSION_COOKIE, path="/")


# --- 今の利用者と管理者 ---


def require_admin() -> None:
    """管理者だけを通す依存関数。

    多人数モードでないときは 404（この住所は多人数モードだけ）。
    今の利用者が管理者でなければ 403「管理者だけが使えます」。
    """
    if not registry.multi_user():
        raise HTTPException(status_code=404, detail="多人数モードではありません")
    user = registry.current_user.get()
    if not isinstance(user, dict) or not user.get("is_admin"):
        raise HTTPException(status_code=403, detail="管理者だけが使えます")


# --- 利用者の置き場（共有契約 C4） ---


def create_user_dirs(name: str) -> None:
    """その人の置き場を作る（`home/.sai`・`projects`・`homes`。0700）。"""
    base = registry.users_root()
    if base is None:
        raise RuntimeError("多人数モードではありません")
    root = base / name
    for path in (root / "home" / ".sai", root / "projects", root / "homes"):
        path.mkdir(parents=True, exist_ok=True)
    # mkdir の既定は umask に左右されるので、0700 にそろえる（親も含む）。
    for path in (
        root,
        root / "home",
        root / "home" / ".sai",
        root / "projects",
        root / "homes",
    ):
        try:
            os.chmod(path, 0o700)
        except OSError:
            pass


def user_public(user: dict) -> dict:
    """利用者の行から、画面に返してよい項目だけを取り出す。"""
    return {
        "name": user.get("name") or "",
        "email": user.get("email") or "",
        "is_admin": bool(user.get("is_admin")),
        "must_change_password": bool(user.get("must_change_password")),
    }
