#!/usr/bin/env python3
from __future__ import annotations
import csv, hashlib, http.server, json, os, random, re, resource, shutil, socketserver, subprocess, tempfile, threading, time, urllib.request
from collections import Counter, OrderedDict, defaultdict
from dataclasses import dataclass, asdict
from pathlib import Path
from typing import Dict, List, Tuple

ROOT = ("OBSERVE", "DERIVE", "COMMIT")
CAP_ROOT = {
    "fs.read": ("OBSERVE",),
    "state.read": ("OBSERVE",),
    "http.get": ("DERIVE", "OBSERVE"),
    "process.run": ("DERIVE", "COMMIT", "OBSERVE"),
    "transform.extract": ("DERIVE",),
    "transform.summarize": ("DERIVE",),
    "fs.write": ("DERIVE", "COMMIT"),
    "state.write": ("DERIVE", "COMMIT"),
    "notify.send": ("DERIVE", "COMMIT"),
}

@dataclass
class Episode:
    idx: int
    task: str
    phase: str
    seq: List[str]
    source: str
    output: str
    url: str = ""
    ancestral: bool = False
    drift: bool = False

@dataclass
class PlanMetrics:
    semantic: int = 0
    recovery: int = 0
    local_repair: int = 0
    structural_change: int = 0
    derived: int = 0
    active: int = 0
    structure_bytes: int = 0
    avg_depth: float = 0.0
    max_depth: int = 0
    uniform: bool = True

class Vocabulary:
    def __init__(self):
        self.derived: Dict[str, List[str]] = OrderedDict()
        self.counter = 0

    def expand_symbol(self, sym: str, stack=None) -> List[str]:
        if stack is None: stack = set()
        if sym in CAP_ROOT:
            return [sym]
        if sym in stack:
            raise ValueError(f"cycle:{sym}")
        if sym not in self.derived:
            raise ValueError(f"unknown:{sym}")
        stack.add(sym)
        out=[]
        for part in self.derived[sym]:
            out.extend(self.expand_symbol(part, stack))
        stack.remove(sym)
        return out

    def expand_root(self, sym: str) -> List[str]:
        caps = self.expand_symbol(sym)
        out=[]
        for c in caps:
            out.extend(CAP_ROOT[c])
        return out

    def all_expansions(self) -> List[Tuple[str,List[str]]]:
        out=[]
        for k in self.derived:
            out.append((k,self.expand_symbol(k)))
        out.sort(key=lambda x:(-len(x[1]),x[0]))
        return out

    def compress(self, seq: List[str], extra=None) -> List[str]:
        expansions = list(extra or []) + self.all_expansions()
        expansions.sort(key=lambda x:(-len(x[1]),x[0]))
        out=[]; i=0
        while i < len(seq):
            hit=None
            for name, ex in expansions:
                n=len(ex)
                if n>=2 and seq[i:i+n]==ex:
                    hit=(name,n); break
            if hit:
                out.append(hit[0]); i += hit[1]
            else:
                out.append(seq[i]); i += 1
        return out

    def depth(self, sym:str, stack=None)->int:
        if sym in CAP_ROOT: return 0
        if stack is None: stack=set()
        if sym in stack: raise ValueError("cycle")
        stack.add(sym)
        d=1+max((self.depth(x,stack) for x in self.derived[sym]), default=0)
        stack.remove(sym)
        return d

    def audit(self)->bool:
        try:
            for name in self.derived:
                roots=self.expand_root(name)
                if not roots or any(r not in ROOT for r in roots): return False
            return True
        except Exception:
            return False

class DRMPlanner:
    def __init__(self, active_cap=7, mdl_threshold=3):
        self.vocab=Vocabulary(); self.active_cap=active_cap; self.mdl_threshold=mdl_threshold
        self.active: OrderedDict[str,List[str]] = OrderedDict()
        self.history: Dict[str,List[str]] = {}
        self.history_version: Dict[str,int] = {}
        self.subseq_users: Dict[Tuple[str,...],set] = defaultdict(set)
        self.version=0
        self.force_ancestral=set()

    def _touch(self, task, seq):
        if task in self.active: del self.active[task]
        self.active[task]=list(seq)
        while len(self.active)>self.active_cap:
            self.active.popitem(last=False)

    def note_subseqs(self, task, seq):
        for n in range(2,min(5,len(seq))+1):
            for i in range(0,len(seq)-n+1):
                self.subseq_users[tuple(seq[i:i+n])].add(task)

    def corpus_cost(self, extra=None):
        return sum(len(self.vocab.compress(s, extra=extra)) for s in self.history.values())

    def maybe_grow(self):
        baseline=self.corpus_cost(); existing={tuple(self.vocab.expand_symbol(k)) for k in self.vocab.derived}
        best=None
        for cand_t, users in self.subseq_users.items():
            if len(users)<2 or cand_t in existing: continue
            cand=list(cand_t)
            definition=self.vocab.compress(cand)
            if len(definition)<=1: continue
            new_cost=sum(len(self.vocab.compress(s, extra=[("__new__",cand)])) for s in self.history.values())
            gain=baseline-new_cost-(len(definition)+1)
            if gain < self.mdl_threshold: continue
            key=(gain,len(users),len(cand))
            if best is None or key>best[0]: best=(key,cand,definition)
        if best is None: return 0
        _,cand,definition=best
        self.vocab.counter += 1
        name=f"d{self.vocab.counter:03d}"
        self.vocab.derived[name]=definition
        return 1

    @staticmethod
    def diff_middle(old,new):
        p=0
        while p<min(len(old),len(new)) and old[p]==new[p]: p+=1
        s=0
        while s<min(len(old)-p,len(new)-p) and old[-1-s]==new[-1-s]: s+=1
        return new[p:len(new)-s if s else len(new)]

    def plan(self, ep:Episode)->PlanMetrics:
        m=PlanMetrics(); seq=ep.seq
        if ep.task in self.active:
            old=self.active[ep.task]
            if old==seq:
                m.semantic=1
            else:
                delta=self.diff_middle(old,seq)
                m.semantic=max(1,len(self.vocab.compress(delta)))
                m.local_repair=1
                m.structural_change += 1
        elif ep.task in self.history:
            old=self.history[ep.task]
            if ep.ancestral or ep.task in self.force_ancestral:
                # Temporarily OBSERVE old developmental state, then DERIVE+COMMIT forward integration.
                m.recovery=1
                m.semantic=max(1,len(self.vocab.compress(seq)))
                m.structural_change += 1
                self.force_ancestral.discard(ep.task)
            elif old != seq:
                delta=self.diff_middle(old,seq)
                m.semantic=max(1,len(self.vocab.compress(delta)))
                m.local_repair=1
                m.structural_change += 1
            else:
                # Canonical task IR is still recoverable without loading an ancestral cognitive state.
                m.semantic=1
        else:
            m.semantic=max(1,len(self.vocab.compress(seq)))
            m.structural_change += 1
        self.version += 1
        self.history[ep.task]=list(seq); self.history_version[ep.task]=self.version
        self.note_subseqs(ep.task,seq)
        grew=self.maybe_grow(); m.structural_change += grew
        self._touch(ep.task,seq)
        m.derived=len(self.vocab.derived); m.active=len(self.active)
        m.uniform=self.vocab.audit()
        ds=[self.vocab.depth(k) for k in self.vocab.derived]
        m.avg_depth=(sum(ds)/len(ds)) if ds else 0.0; m.max_depth=max(ds) if ds else 0
        struct={"root":ROOT,"cap_root":CAP_ROOT,"derived":self.vocab.derived,
                "active":self.active,"history_ptr":self.history_version}
        m.structure_bytes=len(json.dumps(struct,sort_keys=True,separators=(",",":")).encode())
        return m

class BaselinePlanner:
    def __init__(self,kind):
        self.kind=kind; self.seen={}; self.history={}; self.active=OrderedDict(); self.active_cap=7
    @staticmethod
    def diff_middle(old,new): return DRMPlanner.diff_middle(old,new)
    def plan(self,ep:Episode)->PlanMetrics:
        m=PlanMetrics(); seq=ep.seq
        if self.kind=="stateless":
            m.semantic=len(seq)
            m.structure_bytes=0
        elif self.kind=="template_cache":
            if ep.task in self.seen and self.seen[ep.task]==seq: m.semantic=1
            else: m.semantic=len(seq); m.structural_change=1
            self.seen[ep.task]=list(seq)
            m.structure_bytes=len(json.dumps(self.seen,sort_keys=True,separators=(",",":")).encode())
        elif self.kind=="checkpoint_replay":
            if ep.task in self.history:
                old=self.history[ep.task]
                if old==seq: m.semantic=1
                else: m.semantic=max(1,len(self.diff_middle(old,seq))); m.local_repair=1
                if ep.ancestral: m.recovery=1
            else:
                m.semantic=len(seq); m.structural_change=1
            self.history[ep.task]=list(seq)
            m.structure_bytes=len(json.dumps(self.history,sort_keys=True,separators=(",",":")).encode())
        return m

class QuietHandler(http.server.SimpleHTTPRequestHandler):
    def log_message(self,*args): pass

class ThreadingHTTPServer(socketserver.ThreadingMixIn, http.server.HTTPServer): daemon_threads=True

class LiveExecutor:
    def __init__(self,work:Path):
        self.work=work; self.state={"runs":0}; self.commit_count=0; self.root_counts=Counter()
        self.httpd=None; self.thread=None; self.port=None
    def start_server(self):
        handler=lambda *a,**kw: QuietHandler(*a,directory=str(self.work/"web"),**kw)
        self.httpd=ThreadingHTTPServer(("127.0.0.1",0),handler); self.port=self.httpd.server_address[1]
        self.thread=threading.Thread(target=self.httpd.serve_forever,daemon=True); self.thread.start()
    def stop_server(self):
        if self.httpd: self.httpd.shutdown(); self.httpd.server_close()
    def _root(self,*ops): self.root_counts.update(ops)
    def execute(self, ep:Episode):
        ctx={"data":"","source":ep.source,"output":ep.output,"url":ep.url.replace("{PORT}",str(self.port))}
        ok=True; err=""
        try:
            for cap in ep.seq:
                roots=CAP_ROOT[cap]; self._root(*roots)
                if cap=="fs.read":
                    ctx["data"]=(self.work/ctx["source"]).read_text()
                elif cap=="state.read":
                    p=self.work/"state.json"; ctx["data"]=p.read_text() if p.exists() else json.dumps(self.state)
                elif cap=="http.get":
                    req=urllib.request.Request(ctx["url"],headers={"User-Agent":"drm-live-eval/1"})
                    with urllib.request.urlopen(req,timeout=5) as r: ctx["data"]=r.read().decode("utf-8","replace")
                elif cap=="process.run":
                    target=self.work/ctx["source"]
                    cp=subprocess.run(["sha256sum",str(target)],stdout=subprocess.PIPE,stderr=subprocess.PIPE,text=True,timeout=5,check=True)
                    ctx["data"]=cp.stdout.strip()
                elif cap=="transform.extract":
                    txt=re.sub(r"<[^>]+>"," ",ctx["data"]); ctx["data"]=" ".join(txt.split())
                elif cap=="transform.summarize":
                    words=ctx["data"].split(); digest=hashlib.sha256(ctx["data"].encode()).hexdigest()[:12]
                    ctx["data"]=f"words={len(words)} sha={digest} head={' '.join(words[:12])}"
                elif cap=="fs.write":
                    out=self.work/ctx["output"]; out.parent.mkdir(parents=True,exist_ok=True)
                    tmp=out.with_suffix(out.suffix+".candidate"); tmp.write_text(ctx["data"]); os.replace(tmp,out); self.commit_count+=1
                elif cap=="state.write":
                    self.state["runs"]+=1; self.state["last"]=ctx["data"][:120]
                    p=self.work/"state.json"; tmp=self.work/"state.json.candidate"; tmp.write_text(json.dumps(self.state,sort_keys=True)); os.replace(tmp,p); self.commit_count+=1
                elif cap=="notify.send":
                    with (self.work/"notifications.log").open("a") as f: f.write(ctx["data"].replace("\n"," ")[:300]+"\n")
                    self.commit_count+=1
            if "fs.write" in ep.seq:
                ok=(self.work/ctx["output"]).exists() and (self.work/ctx["output"]).stat().st_size>0
            elif "notify.send" in ep.seq:
                ok=(self.work/"notifications.log").exists()
        except Exception as e:
            ok=False; err=f"{type(e).__name__}:{e}"
        return ok,err

def rusage_snapshot():
    a=resource.getrusage(resource.RUSAGE_SELF); b=resource.getrusage(resource.RUSAGE_CHILDREN)
    return (a.ru_utime+b.ru_utime,a.ru_stime+b.ru_stime)

def io_snapshot():
    d={}
    try:
        for line in Path('/proc/self/io').read_text().splitlines():
            k,v=line.split(':',1); d[k.strip()]=int(v.strip())
    except Exception: pass
    return d

def rss_tree_kb():
    try:
        import psutil
        p=psutil.Process(); procs=[p]+p.children(recursive=True); return sum(x.memory_info().rss for x in procs if x.is_running())//1024
    except Exception: return 0

def make_fixtures(work:Path):
    (work/"inputs").mkdir(parents=True,exist_ok=True); (work/"web").mkdir(parents=True,exist_ok=True); (work/"outputs").mkdir(parents=True,exist_ok=True)
    for i in range(12):
        rows=[f"item,{j},value,{i*j+3}" for j in range(1,45)]
        (work/"inputs"/f"report_{i}.csv").write_text("kind,id,label,value\n"+"\n".join(rows)+"\n")
    for i in range(8):
        body=" ".join([f"Story{i}-{j} DRM local systems scheduling repeated task optimization Linux news" for j in range(1,35)])
        (work/"web"/f"news_{i}.html").write_text(f"<html><body><h1>News {i}</h1><p>{body}</p></body></html>")

def make_workload()->List[Episode]:
    eps=[]; idx=0
    def add(task,phase,seq,source="inputs/report_0.csv",output=None,url="",ancestral=False,drift=False):
        nonlocal idx; idx+=1
        eps.append(Episode(idx,task,phase,list(seq),source,output or f"outputs/{task}.txt",url,ancestral,drift))
    motifs={
      "file":["fs.read","transform.summarize","fs.write","notify.send"],
      "file_extract":["fs.read","transform.extract","transform.summarize","fs.write"],
      "hash":["process.run","transform.summarize","fs.write","notify.send"],
      "http":["http.get","transform.extract","transform.summarize","fs.write","notify.send"],
      "state":["state.read","transform.summarize","state.write","notify.send"],
    }
    # Warmup/repetition establishes shared motifs.
    for r in range(3):
        add("daily_file","warmup",motifs["file"],f"inputs/report_{r}.csv")
        add("daily_hash","warmup",motifs["hash"],f"inputs/report_{r+1}.csv")
        add("daily_http","warmup",motifs["http"],url=f"http://127.0.0.1:{{PORT}}/news_{r%3}.html")
        add("daily_state","warmup",motifs["state"])
    # Long-tail unseen compositions sharing pieces.
    combos=[
      ["fs.read","transform.extract","transform.summarize","fs.write","notify.send"],
      ["http.get","transform.extract","transform.summarize","state.write","notify.send"],
      ["process.run","transform.extract","transform.summarize","fs.write"],
      ["state.read","transform.extract","transform.summarize","fs.write","notify.send"],
      ["fs.read","transform.summarize","state.write","notify.send"],
    ]
    for i in range(25):
        seq=combos[i%len(combos)]
        src=f"inputs/report_{i%12}.csv"; url=f"http://127.0.0.1:{{PORT}}/news_{i%8}.html"
        add(f"novel_{i:02d}","novel",seq,src,url=url)
    # Reuse novel tasks after the vocabulary has developed.
    for i in range(12):
        ep=next(e for e in eps if e.task==f"novel_{i:02d}")
        add(ep.task,"repeat",ep.seq,ep.source,ep.output,ep.url)
    # Drift/local repairs.
    add("daily_file","drift",["fs.read","transform.extract","transform.summarize","fs.write","notify.send"],"inputs/report_9.csv",drift=True)
    add("daily_http","drift",["http.get","transform.extract","transform.summarize","state.write","notify.send"],url="http://127.0.0.1:{PORT}/news_7.html",drift=True)
    add("daily_hash","drift",["process.run","transform.summarize","state.write","notify.send"],"inputs/report_10.csv",drift=True)
    # Push hot context forward so old tasks are archived.
    for i in range(7):
        add(f"tail_{i}","evict",combos[i%len(combos)],f"inputs/report_{(i+2)%12}.csv",url=f"http://127.0.0.1:{{PORT}}/news_{i%8}.html")
    # Historical state recovery then immediate repeat should stop recovering.
    for task in ["daily_http","daily_file","daily_hash"]:
        original=next(e for e in eps if e.task==task)
        add(task,"ancestral",original.seq,original.source,original.output,original.url,ancestral=True)
        add(task,"post_recovery",original.seq,original.source,original.output,original.url)
    return eps

def run_live(outdir:Path):
    work=outdir/"workspace"; shutil.rmtree(work,ignore_errors=True); make_fixtures(work)
    ex=LiveExecutor(work); ex.start_server(); eps=make_workload(); drm=DRMPlanner(active_cap=7,mdl_threshold=3)
    rows=[]; trace=[]; vocab_rows=[]
    try:
        for ep in eps:
            before_ru=rusage_snapshot(); before_io=io_snapshot(); t0=time.perf_counter_ns(); peak=rss_tree_kb(); stop=False
            # lightweight sampler captures Chromium descendants while command runs
            samples=[]
            def sampler():
                while not stop:
                    samples.append(rss_tree_kb()); time.sleep(0.004)
            th=threading.Thread(target=sampler,daemon=True); th.start()
            pm=drm.plan(ep)
            ok,err=ex.execute(ep)
            stop=True; th.join(timeout=.1)
            wall=(time.perf_counter_ns()-t0)/1e6; after_ru=rusage_snapshot(); after_io=io_snapshot(); peak=max([peak]+samples+[rss_tree_kb()])
            row={"episode":ep.idx,"task":ep.task,"phase":ep.phase,"success":int(ok),"error":err,
                 "wall_ms":round(wall,3),"cpu_user_ms":round((after_ru[0]-before_ru[0])*1000,3),"cpu_sys_ms":round((after_ru[1]-before_ru[1])*1000,3),
                 "peak_rss_kb":peak,"read_bytes":after_io.get("read_bytes",0)-before_io.get("read_bytes",0),"write_bytes":after_io.get("write_bytes",0)-before_io.get("write_bytes",0),
                 "semantic":pm.semantic,"recovery":pm.recovery,"local_repair":pm.local_repair,"structural_change":pm.structural_change,
                 "derived":pm.derived,"active":pm.active,"structure_bytes":pm.structure_bytes,"avg_depth":round(pm.avg_depth,3),"max_depth":pm.max_depth,"uniform":int(pm.uniform),
                 "model_calls":0,"model_input_tokens":0,"model_output_tokens":0}
            rows.append(row)
            trace.append({"episode":ep.idx,"phase":ep.phase,"semantic":pm.semantic,"derived":pm.derived,"structure_bytes":pm.structure_bytes,"avg_depth":pm.avg_depth,"recovery":pm.recovery,"local_repair":pm.local_repair})
        # Full vocabulary audit table.
        for name,definition in drm.vocab.derived.items():
            caps=drm.vocab.expand_symbol(name); roots=drm.vocab.expand_root(name)
            vocab_rows.append({"name":name,"definition":" > ".join(definition),"capability_expansion":" > ".join(caps),"root_expansion":" > ".join(roots),"depth":drm.vocab.depth(name),"uniform":int(all(x in ROOT for x in roots))})
    finally: ex.stop_server()
    outdir.mkdir(parents=True,exist_ok=True)
    def write_csv(path,items):
        if not items: return
        with path.open('w',newline='') as f:
            w=csv.DictWriter(f,fieldnames=list(items[0])); w.writeheader(); w.writerows(items)
    write_csv(outdir/"live_trace.csv",rows); write_csv(outdir/"growth_trace.csv",trace); write_csv(outdir/"vocabulary_audit.csv",vocab_rows)
    # Baseline planner-only evaluation over identical task sequence.
    base=[]
    for kind in ["stateless","template_cache","checkpoint_replay"]:
        p=BaselinePlanner(kind); sem=rec=rep=chg=0; t0=time.perf_counter_ns()
        last=None
        for ep in eps:
            x=p.plan(ep); sem+=x.semantic; rec+=x.recovery; rep+=x.local_repair; chg+=x.structural_change; last=x
        base.append({"system":kind,"episodes":len(eps),"semantic_total":sem,"semantic_mean":round(sem/len(eps),4),"recoveries":rec,"local_repairs":rep,"structural_changes":chg,
                     "final_structure_bytes":last.structure_bytes if last else 0,"planner_wall_ms":round((time.perf_counter_ns()-t0)/1e6,3)})
    drm_sem=sum(r["semantic"] for r in rows)
    base.append({"system":"drm_odc","episodes":len(eps),"semantic_total":drm_sem,"semantic_mean":round(drm_sem/len(eps),4),"recoveries":sum(r["recovery"] for r in rows),"local_repairs":sum(r["local_repair"] for r in rows),
                 "structural_changes":sum(r["structural_change"] for r in rows),"final_structure_bytes":rows[-1]["structure_bytes"],"planner_wall_ms":"included_in_live"})
    write_csv(outdir/"baseline_comparison.csv",base)
    phases=[]
    for phase in sorted(set(r["phase"] for r in rows), key=lambda x:[r["phase"] for r in rows].index(x)):
        rr=[r for r in rows if r["phase"]==phase]
        phases.append({"phase":phase,"n":len(rr),"success_rate":round(sum(x["success"] for x in rr)/len(rr),4),"semantic_mean":round(sum(x["semantic"] for x in rr)/len(rr),4),
                       "wall_ms_mean":round(sum(x["wall_ms"] for x in rr)/len(rr),3),"cpu_ms_mean":round(sum(x["cpu_user_ms"]+x["cpu_sys_ms"] for x in rr)/len(rr),3),
                       "peak_rss_kb_max":max(x["peak_rss_kb"] for x in rr),"derived_end":rr[-1]["derived"],"structure_bytes_end":rr[-1]["structure_bytes"],"recoveries":sum(x["recovery"] for x in rr),"repairs":sum(x["local_repair"] for x in rr)})
    write_csv(outdir/"phase_summary.csv",phases)
    summary={
      "platform": {"uname":" ".join(os.uname()),"python":os.sys.version.split()[0],"chromium":subprocess.run(["chromium","--version"],capture_output=True,text=True).stdout.strip()},
      "episodes":len(rows),"success_rate":sum(r["success"] for r in rows)/len(rows),"total_wall_ms":sum(r["wall_ms"] for r in rows),
      "semantic_total":drm_sem,"semantic_mean":drm_sem/len(rows),"derived_final":len(drm.vocab.derived),"structure_bytes_final":rows[-1]["structure_bytes"],
      "uniform_vocabulary":all(r["uniform"] for r in rows) and drm.vocab.audit(),"root_vocabulary":list(ROOT),"root_counts":dict(ex.root_counts),"commits":ex.commit_count,
      "recoveries":sum(r["recovery"] for r in rows),"local_repairs":sum(r["local_repair"] for r in rows),"model_calls":0,
      "note":"Live Linux filesystem, process spawning, local HTTP, state and transactional writes; semantic planner is deterministic and does not invoke Ollama in this run. Chromium was detected but excluded because headless Chromium hangs under this sandbox DBus environment."
    }
    (outdir/"summary.json").write_text(json.dumps(summary,indent=2,sort_keys=True))
    print(json.dumps(summary,indent=2,sort_keys=True))

if __name__=='__main__':
    out=Path(os.environ.get('DRM_OUT','/tmp/drm-live-results'))
    run_live(out)
