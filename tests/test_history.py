import importlib.machinery
import importlib.util
import os
import pathlib
import sqlite3
import tempfile
import time
import unittest
from unittest.mock import patch

ROOT=pathlib.Path(__file__).resolve().parents[1]
P=ROOT/"deploy/libexec/vpn-ui-history"
loader=importlib.machinery.SourceFileLoader("vpn_ui_history",str(P))
spec=importlib.util.spec_from_loader(loader.name,loader)
h=importlib.util.module_from_spec(spec)
loader.exec_module(h)

class TestHistory(unittest.TestCase):
    def test_valid_peer_is_not_false_online(self):
        now=int(time.time())
        data={"public_key":"keyabc","vpn_ip":"10.66.66.4","name":"test","latest_handshake":now-350}
        r=h.valid_peer(data,"wireguard",now)
        self.assertEqual(r["online"],0)
        self.assertIsNone(r["ping_ok"])
        data["latest_handshake"]=now-10
        self.assertEqual(h.valid_peer(data,"wireguard",now)["online"],1)
        self.assertEqual(h.valid_peer({"ip":"100.125.53.1","online":True},"tailscale",now)["online"],1)
    def test_persistent_history_and_range(self):
        with tempfile.TemporaryDirectory() as temp:
            with patch.dict(os.environ,{"VPN_UI_HISTORY_DB":str(pathlib.Path(temp)/"samples.db")}):
                self.assertFalse(h.query(24)["configured"])
                now=int(time.time())//300*300
                with h.connect() as db:
                    for offset in (0,3600,25*3600):
                        db.execute("""INSERT INTO samples
                        (ts,provider,peer_id,label,ip,online,ping_ok,rtt_ms,path,relay)
                        VALUES (?,?,?,?,?,?,?,?,?,?)""",
                        (now-offset,"tailscale","100.125.53.1","garage-pc",
                         "100.125.53.1",1,1,150.0,"DERP","ams"))
                one=h.query(24,"tailscale","100.125.53.1")
                week=h.query(168,"tailscale","100.125.53.1")
                self.assertEqual(len(one["points"]),2)
                self.assertEqual(len(week["points"]),3)
                self.assertEqual(one["peers"][0]["probes"],2)
                self.assertEqual(one["points"][0]["relay"],"ams")
                with self.assertRaises(ValueError):
                    h.query(1000)
                with self.assertRaises(ValueError):
                    h.query(24,"not-a-provider")

    def test_sampler_never_requires_mutating_api(self):
        now=int(time.time())
        calls=[]
        def fake(route,body=None,timeout=12):
            calls.append((route,body))
            if route.endswith("/status"):
                if "/tailscale/" in route:
                    return {"peers":[{"ip":"100.125.53.1","hostname":"garage-pc","online":True}]}
                return {"peers":[{"public_key":"abc", "vpn_ip":"10.66.66.6","latest_handshake":now}]}
            if route.endswith("/overlay-ping"):
                return {"ok":True,"avg_ms":171,"path":"DERP","relay":"ams"}
            return {"ok":False,"avg_ms":None}
        with tempfile.TemporaryDirectory() as temp:
            with patch.dict(os.environ,{"VPN_UI_HISTORY_DB":str(pathlib.Path(temp)/"samples.db")}):
                with patch.object(h,"request_json",side_effect=fake):
                    self.assertEqual(h.collect(),0)
                result=h.query(24)
                self.assertEqual(len(result["peers"]),4)
                self.assertTrue(all(route.startswith("/api/") for route,_ in calls))
                self.assertTrue(all("/manage" not in route and "/settings" not in route
                                    for route,_ in calls))
                self.assertTrue(any(x["provider"]=="tailscale" for x in result["peers"]))

if __name__ == "__main__":
    unittest.main()
