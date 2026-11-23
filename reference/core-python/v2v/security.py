"""Layer 2 (security): a toy PKI with pseudonym certificates, signing, verification
and misbehavior-driven revocation.

Real V2V uses IEEE 1609.2: ECDSA signatures and pseudonym certificates issued by a
Security Credential Management System (SCMS). Here every certificate holds an HMAC
key that only the issuing CA and the holder know, and verifiers ask the CA for it.
The *flow* is the same: unknown, forged, tampered or revoked = dropped.
The randomness is seeded for reproducible demos; it is NOT cryptographic.
"""

import hashlib
import hmac
import random
from dataclasses import dataclass

from . import config
from .messages import BSM


@dataclass(frozen=True)
class Credential:
    cert_id: str
    key: bytes


@dataclass(frozen=True)
class Packet:
    payload: bytes
    cert_id: str
    sig: str


def _mac(key: bytes, payload: bytes) -> str:
    return hmac.new(key, payload, hashlib.sha256).hexdigest()


class CertificateAuthority:
    def __init__(self, rng: random.Random, revoke_after: int = config.REVOKE_AFTER_REPORTS):
        self._rng = rng
        self._keys: dict[str, bytes] = {}
        self.revoked: set[str] = set()
        self.reports: dict[str, set[str]] = {}
        self.revoke_after = revoke_after

    def issue(self, count: int = 1) -> list[Credential]:
        creds = []
        for _ in range(count):
            cid = f"{self._rng.getrandbits(32):08x}"
            key = self._rng.randbytes(32)
            self._keys[cid] = key
            creds.append(Credential(cid, key))
        return creds

    def lookup(self, cert_id: str):
        if cert_id in self.revoked:
            return None
        return self._keys.get(cert_id)

    def report(self, cert_id: str, reporter: str, reason: str) -> bool:
        """Misbehavior report. Returns True if this report caused a revocation."""
        if cert_id not in self._keys or cert_id in self.revoked:
            return False
        self.reports.setdefault(cert_id, set()).add(reporter)
        if len(self.reports[cert_id]) >= self.revoke_after:
            self.revoked.add(cert_id)
            return True
        return False


class Signer:
    def __init__(self, creds: list[Credential], rotate_every: float = config.PSEUDONYM_ROTATE_S):
        self.creds = creds
        self.rotate_every = rotate_every

    def current(self, now: float) -> Credential:
        return self.creds[int(now // self.rotate_every) % len(self.creds)]

    @staticmethod
    def sign(msg: BSM, cred: Credential) -> Packet:
        raw = msg.encode()
        return Packet(raw, cred.cert_id, _mac(cred.key, raw))


class Verifier:
    def __init__(self, ca: CertificateAuthority, enabled: bool = True):
        self.ca = ca
        self.enabled = enabled

    def verify(self, pkt: Packet):
        """Returns (BSM, None) if accepted, (None, reason) if dropped."""
        if self.enabled:
            key = self.ca.lookup(pkt.cert_id)
            if key is None:
                return None, ("revoked certificate" if pkt.cert_id in self.ca.revoked
                              else "unknown certificate")
            if not hmac.compare_digest(_mac(key, pkt.payload), pkt.sig):
                return None, "bad signature"
        try:
            msg = BSM.decode(pkt.payload)
        except (ValueError, TypeError):
            return None, "malformed message"
        if self.enabled and msg.temp_id != pkt.cert_id:
            return None, "temp ID does not match certificate"
        return msg, None
