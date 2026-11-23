import math
import random
import unittest

from v2v import config
from v2v.geometry import box, polygons_overlap, rect, segment_hits_polygon
from v2v.ldm import LocalDynamicMap
from v2v.messages import BSM
from v2v.prediction import first_conflict, trajectory
from v2v.radio import Frame
from v2v.behavior import BehaviorMonitor
from v2v.scenarios import blind_intersection, emergency_brake, ghost_attack, highway_relay, mixed_traffic
from v2v.security import CertificateAuthority, Packet, Signer, Verifier


def bsm(**kw):
    base = dict(temp_id="aa", t=0.0, x=0.0, y=0.0, speed=10.0, heading=0.0,
                yaw_rate=0.0, accel=0.0, brake_on=False)
    base.update(kw)
    return BSM(**base)


class SecurityTests(unittest.TestCase):
    def setUp(self):
        self.ca = CertificateAuthority(random.Random(1))
        self.cred = self.ca.issue(1)[0]
        self.verifier = Verifier(self.ca)

    def test_valid_message_accepted(self):
        pkt = Signer.sign(bsm(temp_id=self.cred.cert_id), self.cred)
        msg, why = self.verifier.verify(pkt)
        self.assertIsNone(why)
        self.assertEqual(msg.temp_id, self.cred.cert_id)

    def test_tampered_payload_rejected(self):
        pkt = Signer.sign(bsm(temp_id=self.cred.cert_id), self.cred)
        bad = Packet(pkt.payload.replace(b'"speed":10.0', b'"speed":30.0'), pkt.cert_id, pkt.sig)
        self.assertEqual(self.verifier.verify(bad), (None, "bad signature"))

    def test_unknown_certificate_rejected(self):
        pkt = Packet(bsm(temp_id="ffffffff").encode(), "ffffffff", "00")
        self.assertEqual(self.verifier.verify(pkt)[1], "unknown certificate")

    def test_temp_id_must_match_certificate(self):
        pkt = Signer.sign(bsm(temp_id="someone-else"), self.cred)
        self.assertEqual(self.verifier.verify(pkt)[1], "temp ID does not match certificate")

    def test_revocation(self):
        self.ca.report(self.cred.cert_id, "CarA", "test")
        self.ca.report(self.cred.cert_id, "CarB", "test")
        self.assertTrue(self.ca.report(self.cred.cert_id, "CarC", "test"))
        pkt = Signer.sign(bsm(temp_id=self.cred.cert_id), self.cred)
        self.assertEqual(self.verifier.verify(pkt)[1], "revoked certificate")

    def test_pseudonym_rotation(self):
        creds = self.ca.issue(3)
        s = Signer(creds, rotate_every=300)
        self.assertNotEqual(s.current(0).cert_id, s.current(301).cert_id)


class PlausibilityTests(unittest.TestCase):
    def test_checks(self):
        ldm = LocalDynamicMap()
        self.assertIn("speed", ldm.check(bsm(speed=120), now=0))
        self.assertIn("acceleration", ldm.check(bsm(accel=-30), now=0))
        self.assertEqual(ldm.check(bsm(t=0.0), now=3.0), "stale")
        self.assertIsNone(ldm.check(bsm(t=0.0), now=1.0))  # old but fine for relayed data
        self.assertEqual(ldm.check(bsm(t=5.0), now=1.0), "timestamp in the future")
        ldm.update(bsm(t=1.0), now=1.0)
        self.assertEqual(ldm.check(bsm(t=1.0), now=1.1), "duplicate")
        self.assertIn("jump", ldm.check(bsm(t=1.1, x=50), now=1.1))
        self.assertIsNone(ldm.check(bsm(t=1.1, x=1.0), now=1.1))

    def test_eviction(self):
        ldm = LocalDynamicMap(ttl=1.0)
        ldm.update(bsm(), now=0.0)
        ldm.evict(now=0.9)
        self.assertIn("aa", ldm.entries)
        ldm.evict(now=1.5)
        self.assertNotIn("aa", ldm.entries)


class GeometryPredictionTests(unittest.TestCase):
    def test_overlap(self):
        self.assertTrue(polygons_overlap(box(0, 0, 0, 4.5, 1.8), box(3, 0, math.pi / 2, 4.5, 1.8)))
        self.assertFalse(polygons_overlap(box(0, 0, 0, 4.5, 1.8), box(6, 0, 0, 4.5, 1.8)))

    def test_line_of_sight(self):
        self.assertTrue(segment_hits_polygon((-10, 0), (10, 0), rect(-1, -1, 1, 1)))
        self.assertFalse(segment_hits_polygon((-10, 5), (10, 5), rect(-1, -1, 1, 1)))

    def test_braking_car_stops(self):
        tr = trajectory(0, 0, 0, 10.0, -5.0, 0.0, horizon=5.0)
        self.assertAlmostEqual(tr[-1][1], tr[-15][1])  # stopped, not reversing
        self.assertLess(tr[-1][1], 12)

    def test_turning_quarter_circle(self):
        w = math.pi / 2 / 4.0  # 90 deg in 4 s
        tr = trajectory(0, 0, 0, 10.0, 0.0, w, horizon=4.0, step=0.01)
        self.assertAlmostEqual(tr[-1][3], math.pi / 2, places=2)
        r = 10.0 / w
        self.assertAlmostEqual(tr[-1][1], r, delta=0.3)
        self.assertAlmostEqual(tr[-1][2], r, delta=0.3)

    def test_crossing_conflict(self):
        a = trajectory(-20, 0, 0, 10, 0, 0)
        b = trajectory(0, -20, math.pi / 2, 10, 0, 0)
        ttc = first_conflict(a, (4.5, 1.8), b, (4.5, 1.8))
        self.assertIsNotNone(ttc)
        self.assertLess(ttc, 2.0)


class BehaviorTests(unittest.TestCase):
    def test_speeding_and_harsh_events(self):
        mon = BehaviorMonitor(speed_limit=lambda x, y: 50 / 3.6)
        me = bsm(temp_id="me", speed=10.0)
        for i in range(40):                     # peer at 80 km/h in a 50 zone for 4 s
            t = i * 0.1
            peer = bsm(temp_id="p1", t=t, x=20 + 22.2 * t, speed=22.2, accel=-5.0 if i == 30 else 0.0)
            mon.observe(t, 0.1, bsm(temp_id="me", t=t, x=10 * t, speed=10.0), {"p1": peer})
        kinds = [e[2] for e in mon.events if e[1] == "p1"]
        self.assertIn("speeding", kinds)
        self.assertIn("harsh_brake", kinds)
        self.assertGreater(mon.profiles["p1"].over_limit_s, 1.5)
        self.assertEqual(mon.profiles["self"].over_limit_s, 0)

    def test_overtake_in_oncoming_lane(self):
        mon = BehaviorMonitor()
        for i in range(100):                    # p1 passes me on my left at +8 m/s
            t = i * 0.1
            me = bsm(temp_id="me", t=t, x=20 * t, y=-1.75, speed=20.0)
            p1 = bsm(temp_id="p1", t=t, x=-40 + 28 * t, y=1.75, speed=28.0)
            mon.observe(t, 0.1, me, {"p1": p1})
        ev = [e for e in mon.events if e[2] == "overtake"]
        self.assertEqual(len(ev), 1)
        self.assertEqual((ev[0][1], ev[0][5], ev[0][4]), ("p1", "self", 1.0))  # passer, passed, oncoming

    def test_mixed_traffic_report(self):
        w = mixed_traffic()
        w.run()
        self.assertFalse(w.collisions)
        prof = w.behavior_report()["CarA"]["profiles"]
        self.assertGreater(prof["Speeder"]["over_limit_s"], 20)
        self.assertEqual(prof["Speeder"]["oncoming_overtakes"], 3)
        self.assertGreater(prof["Tailgater"]["tailgate_s"], 20)
        self.assertEqual((prof["Jerky"]["harsh_brakes"], prof["Jerky"]["harsh_accels"]), (1, 1))
        self.assertEqual(prof["CarA"]["score"], 100)
        self.assertEqual(prof["Truck"]["score"], 100)


class ScenarioTests(unittest.TestCase):
    def test_blind_intersection(self):
        self.assertTrue(blind_intersection(v2v=False).run()["collisions"])
        s = blind_intersection(v2v=True).run()
        self.assertFalse(s["collisions"])
        self.assertGreater(s["min_gap_m"], 8)
        self.assertEqual(s["min_speed_kmh"]["CarB"], 50.0)  # priority car never brakes

    def test_blind_intersection_with_heavy_loss(self):
        for seed in range(1, 6):
            self.assertFalse(blind_intersection(v2v=True, loss=0.6, seed=seed).run()["collisions"])

    def test_emergency_brake(self):
        self.assertTrue(emergency_brake(v2v=False).run()["collisions"])
        for loss in (0.0, 0.3, 0.6):
            self.assertFalse(emergency_brake(v2v=True, loss=loss).run()["collisions"])

    def test_ghost_attack(self):
        w = ghost_attack(v2v=True)
        s = w.run()
        self.assertEqual(s["min_speed_kmh"]["CarA"], 50.0)   # no phantom braking
        a = w.by_name("CarA")
        self.assertTrue(a.obu.blocked)                       # insider blocked locally at once
        self.assertEqual(len(s["revoked"]), 0)               # one reporter alone cannot revoke
        s_off = ghost_attack(v2v=True, security=False).run()
        self.assertLess(s_off["min_speed_kmh"]["CarA"], 40)  # attack works without security

    def test_relay_reaches_beyond_radio_range(self):
        w = highway_relay(v2v=True)
        while w.t < 1.0:
            w.step()
        a = w.by_name("CarA")
        stop = w.by_name("Stop3")
        self.assertGreater(stop.x - a.x, 1000)               # far beyond 300 m radio range
        far = [w._cert_owner[tid] for tid, h in a.obu.ldm.hops.items()
               if tid in a.obu.ldm.entries and h >= 2]
        self.assertIn("Stop3", far)
        self.assertEqual(a.obu.state, "ADVISE")

    def test_relay_highway_outcomes(self):
        self.assertTrue(highway_relay(v2v=False).run()["collisions"])
        direct = highway_relay(v2v=True, relay=False).run()
        relay = highway_relay(v2v=True, relay=True).run()
        self.assertFalse(direct["collisions"])
        self.assertFalse(relay["collisions"])
        self.assertLess(relay["max_brake_ms2"]["CarA"], direct["max_brake_ms2"]["CarA"] - 1.0)

    def test_relay_cannot_alter_message(self):
        ca = CertificateAuthority(random.Random(1))
        cred = ca.issue(1)[0]
        pkt = Signer.sign(bsm(temp_id=cred.cert_id), cred)
        forged = Packet(pkt.payload.replace(b'"speed":10.0', b'"speed":0.0'), pkt.cert_id, pkt.sig)
        w = blind_intersection(v2v=True)
        obu = w.by_name("CarA").obu
        obu.receive(Frame(forged, hops=3, last_hop=(500.0, 0.0)), now=0.0)
        self.assertNotIn(cred.cert_id, obu.ldm.entries)
        self.assertEqual(obu.outbox, [])                     # and it is not forwarded

    def test_revocation_needs_independent_reporters(self):
        ca = CertificateAuthority(random.Random(1))
        cid = ca.issue(1)[0].cert_id
        self.assertFalse(ca.report(cid, "CarA", "x"))
        self.assertFalse(ca.report(cid, "CarA", "x"))       # same reporter twice: still no
        self.assertFalse(ca.report(cid, "CarB", "x"))
        self.assertTrue(ca.report(cid, "CarC", "x"))        # third independent reporter
        self.assertIn(cid, ca.revoked)

    def test_message_flood_is_rejected(self):
        ca = CertificateAuthority(random.Random(1))
        cred = ca.issue(1)[0]
        w = blind_intersection(v2v=True)
        obu = w.by_name("CarA").obu
        obu.ca = ca
        obu.verifier.ca = ca
        for i in range(5):                                   # 5 messages 10 ms apart = 100 msg/s
            m = bsm(temp_id=cred.cert_id, t=1.0 + i * 0.01, x=-40 + i * 0.1)
            obu.receive(Frame(Signer.sign(m, cred), 0, (0.0, 0.0)), now=1.0 + i * 0.01)
        self.assertIn(cred.cert_id, obu.blocked)
        self.assertNotIn(cred.cert_id, obu.ldm.entries)

    def test_yield_failsafe_when_target_lost(self):
        w = blind_intersection(v2v=True)
        while w.t < 2.5:
            w.step()
        a = w.by_name("CarA")
        self.assertEqual(a.obu.state, "YIELD")
        w.channel.loss = 1.0                   # radio blackout
        while w.t < 4.0:
            w.step()
        self.assertEqual(a.obu.state, "YIELD")  # did not drive on blind
        self.assertLessEqual(a.accel, config.FAILSAFE_BRAKE)


if __name__ == "__main__":
    unittest.main()
