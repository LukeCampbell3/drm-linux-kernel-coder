from __future__ import annotations
from dataclasses import dataclass, field
from collections import OrderedDict, defaultdict
from typing import Dict, List, Tuple, Set, Iterable
import csv, random, statistics, math, os

CANONICAL = [
    'process.spawn','process.wait','fs.read','fs.write','fs.move',
    'http.get','browser.navigate','text.extract','text.summarize',
    'state.read','state.write','notify.send','schedule.trigger','network.check'
]
CANON = set(CANONICAL)

@dataclass(frozen=True)
class Episode:
    task: str
    seq: Tuple[str, ...]
    phase: str
    needs_ancestry: bool = False


def diff_middle(old: Tuple[str,...], new: Tuple[str,...]) -> Tuple[str,...]:
    p=0
    while p < min(len(old),len(new)) and old[p]==new[p]: p+=1
    s=0
    while s < min(len(old)-p,len(new)-p) and old[-1-s]==new[-1-s]: s+=1
    end = len(new)-s if s else len(new)
    return new[p:end]

class StatelessPlanner:
    name='stateless_planner'
    def step(self, ep):
        return {'semantic':len(ep.seq),'recovery':0,'storage':0,'active':0,'derived':0,'change':0}

class ExactReplay:
    name='task_checkpoint_replay'
    def __init__(self): self.cache={}
    def step(self, ep):
        prev=self.cache.get(ep.task)
        if prev == ep.seq:
            cost=1; ch=0
        else:
            cost=len(ep.seq); ch=1
            self.cache[ep.task]=ep.seq
        storage=sum(len(v) for v in self.cache.values())
        return {'semantic':cost,'recovery':0,'storage':storage,'active':len(self.cache),'derived':0,'change':ch}

class FlatTemplate:
    name='flat_template_cache'
    def __init__(self): self.templates=set()
    def step(self, ep):
        if ep.seq in self.templates:
            cost=1; ch=0
        else:
            cost=len(ep.seq); ch=1; self.templates.add(ep.seq)
        storage=sum(len(v) for v in self.templates)
        return {'semantic':cost,'recovery':0,'storage':storage,'active':len(self.templates),'derived':0,'change':ch}

@dataclass
class Derived:
    name: str
    definition: Tuple[str,...]  # canonical or lower derived symbols

class DRM:
    name='drm_hierarchical'
    def __init__(self, active_cap=12, max_derived=16):
        self.active_cap=active_cap
        self.max_derived=max_derived
        self.active: OrderedDict[str, Tuple[Tuple[str,...],Tuple[str,...]]] = OrderedDict()
        self.history: Dict[str, Tuple[str,...]]={}
        self.derived: Dict[str,Derived]={}
        self.exp_cache: Dict[str,Tuple[str,...]]={}
        self.subseq_users: Dict[Tuple[str,...],Set[str]]=defaultdict(set)
        self.version=0
        self.derived_counter=0
        self.history_refs=0
        self.total_recoveries=0
        self.structural_changes=0
        self.uniformity_failures=0

    def expand_symbol(self,sym,stack=None):
        if sym in CANON: return (sym,)
        if sym in self.exp_cache: return self.exp_cache[sym]
        if sym not in self.derived: raise ValueError(f'unknown vocabulary symbol {sym}')
        stack=set() if stack is None else set(stack)
        if sym in stack: raise ValueError(f'cycle at {sym}')
        stack.add(sym)
        out=[]
        for x in self.derived[sym].definition:
            out.extend(self.expand_symbol(x,stack))
        self.exp_cache[sym]=tuple(out)
        return tuple(out)

    def audit_uniformity(self):
        try:
            for name in self.derived:
                ex=self.expand_symbol(name)
                if not ex or any(x not in CANON for x in ex): return False
            for _,(_,rep) in self.active.items():
                ex=[]
                for s in rep: ex.extend(self.expand_symbol(s))
                if any(x not in CANON for x in ex): return False
            return True
        except Exception:
            return False

    def vocab_expansions(self):
        return sorted(((name,self.expand_symbol(name)) for name in self.derived), key=lambda x:(-len(x[1]),x[0]))

    def compress(self, seq: Tuple[str,...]) -> Tuple[str,...]:
        if not seq: return tuple()
        exps=self.vocab_expansions()
        out=[]; i=0
        while i<len(seq):
            match=None
            for name,ex in exps:
                L=len(ex)
                if L>=2 and tuple(seq[i:i+L])==ex:
                    match=(name,L); break
            if match:
                out.append(match[0]); i+=match[1]
            else:
                out.append(seq[i]); i+=1
        return tuple(out)

    def note_subseqs(self, task, seq):
        n=len(seq)
        for L in range(2,min(6,n+1)):
            for i in range(n-L+1):
                self.subseq_users[tuple(seq[i:i+L])].add(task)

    def maybe_add_derived(self):
        if len(self.derived)>=self.max_derived: return 0
        existing={self.expand_symbol(name) for name in self.derived}
        base_exps=self.vocab_expansions()
        baseline_corpus=sum(len(self.compress(seq)) for seq in self.history.values())

        def compress_with_extra(sequence, extra_seq):
            exps=[('__candidate__',extra_seq)]+base_exps
            exps.sort(key=lambda x:(-len(x[1]),x[0]))
            out=[]; i=0
            while i<len(sequence):
                hit=None
                for name,ex in exps:
                    L=len(ex)
                    if L>=2 and tuple(sequence[i:i+L])==ex:
                        hit=(name,L); break
                if hit:
                    out.append(hit[0]); i+=hit[1]
                else:
                    out.append(sequence[i]); i+=1
            return tuple(out)

        candidates=[]
        for seq,users in self.subseq_users.items():
            if len(users)<2 or seq in existing: continue
            definition=self.compress(seq)
            if len(definition)<=1:
                continue
            # Global MDL gate: a word is admitted only if adding it reduces the
            # description length of the whole learned task corpus after paying for
            # its own definition and registry entry. This suppresses overlapping
            # synonyms/prefixes that look locally useful but inflate the vocabulary.
            new_corpus=sum(len(compress_with_extra(task_seq,seq)) for task_seq in self.history.values())
            gain=baseline_corpus-new_corpus-(len(definition)+1)
            if gain>=3:
                candidates.append((gain,len(users),len(seq),seq,definition))
        if not candidates: return 0
        candidates.sort(reverse=True)
        _,_,_,seq,definition=candidates[0]
        self.derived_counter+=1
        name=f'd{self.derived_counter:02d}'
        if any(x not in CANON and x not in self.derived for x in definition):
            raise AssertionError('non-uniform derived definition')
        self.derived[name]=Derived(name,definition)
        self.exp_cache.clear()
        changed=0
        for task,(canonical,rep) in list(self.active.items()):
            newrep=self.compress(canonical)
            if len(newrep)<len(rep):
                self.active[task]=(canonical,newrep); changed+=1
        return 1+changed

    def touch_active(self, task, seq):
        rep=self.compress(seq)
        if task in self.active: del self.active[task]
        self.active[task]=(seq,rep)
        ev=0
        while len(self.active)>self.active_cap:
            self.active.popitem(last=False); ev+=1
        return ev

    def step(self, ep):
        self.version+=1
        recovery=0; change=0; local_repair=0
        if ep.task in self.active:
            old,rep=self.active[ep.task]
            if old==ep.seq:
                semantic=1
            else:
                mid=diff_middle(old,ep.seq)
                semantic=1+len(self.compress(mid))
                local_repair=1; change+=1
        elif self.history.get(ep.task)==ep.seq:
            # A persisted Task IR can normally be forward-compiled through today's
            # vocabulary without hydrating an old DRM. Ancestral execution is reserved
            # for episodes whose semantic context is explicitly unavailable today.
            if ep.needs_ancestry:
                semantic=2; recovery=1; self.total_recoveries+=1; change+=1
            else:
                semantic=1; change+=1
        elif ep.task in self.history:
            # Workflow drift: reuse historical IR and repair only the changed region.
            # Hydrate ancestry only when the test marks the old semantic context as needed.
            mid=diff_middle(self.history[ep.task],ep.seq)
            repaired=self.compress(mid)
            if ep.needs_ancestry:
                semantic=2+len(repaired); recovery=1; self.total_recoveries+=1
            else:
                semantic=1+len(repaired)
            local_repair=1; change+=1
        else:
            semantic=len(self.compress(ep.seq)); change+=1

        self.history[ep.task]=ep.seq
        self.history_refs=len(self.history)
        self.note_subseqs(ep.task,ep.seq)
        evictions=self.touch_active(ep.task,ep.seq)
        vocab_changes=self.maybe_add_derived()
        change += vocab_changes + evictions
        self.structural_changes += change
        if not self.audit_uniformity(): self.uniformity_failures += 1

        vocab_storage=len(CANONICAL)+sum(len(d.definition) for d in self.derived.values())
        active_storage=sum(len(rep) for _,rep in self.active.values())
        # Immutable lineage is represented as one task-root reference per historical task here;
        # definitions and canonical leaves are content-addressed/shared, so refs are counted separately.
        runtime_storage=vocab_storage+active_storage
        return {'semantic':semantic,'recovery':recovery,'storage':runtime_storage,
                'active':len(self.active),'derived':len(self.derived),'change':change,
                'local_repair':local_repair,'evictions':evictions,
                'uniform':1 if self.audit_uniformity() else 0,
                'vocab_storage':vocab_storage,'active_storage':active_storage,
                'history_refs':self.history_refs}


def build_task_catalog():
    P={x:x for x in CANONICAL}
    motifs={
      'web': (P['process.spawn'],P['browser.navigate'],P['text.extract']),
      'api': (P['network.check'],P['http.get'],P['text.extract']),
      'summ': (P['text.summarize'],P['notify.send']),
      'load': (P['fs.read'],P['text.extract']),
      'persist': (P['fs.write'],P['state.write']),
      'archive': (P['fs.read'],P['fs.move'],P['state.write']),
      'proc': (P['process.spawn'],P['process.wait'],P['state.read']),
    }
    tasks={}
    # Initial tasks (10) - repeated structures create cross-task vocabulary.
    for topic in ['ai','nba','security']:
        tasks[f'news.{topic}']=motifs['web']+motifs['summ']
    for topic in ['weather','stocks']:
        tasks[f'api.{topic}']=motifs['api']+motifs['summ']
    for r in ['daily','weekly']:
        tasks[f'report.{r}']=motifs['load']+(P['state.write'],P['notify.send'])
    tasks['files.archive']=motifs['archive']+(P['notify.send'],)
    tasks['system.health']=motifs['proc']+(P['notify.send'],)
    tasks['repo.sync']=(P['process.spawn'],P['process.wait'],P['fs.write'],P['notify.send'])
    # Expansion: novel compositions of already-seen motifs.
    for topic in ['science','finance','local']:
        tasks[f'research.{topic}']=motifs['web']+(P['fs.write'],)+motifs['summ']
    for topic in ['scores','traffic','packages']:
        tasks[f'api_save.{topic}']=motifs['api']+motifs['persist']+(P['notify.send'],)
    for name in ['invoice','expense','telemetry']:
        tasks[f'process.{name}']=motifs['load']+motifs['persist']+(P['notify.send'],)
    tasks['system.audit']=motifs['proc']+(P['text.summarize'],P['notify.send'])
    tasks['scheduled.news']=(P['schedule.trigger'],)+motifs['web']+motifs['summ']
    tasks['scheduled.report']=(P['schedule.trigger'],)+motifs['load']+(P['state.write'],P['notify.send'])
    return tasks


def build_workload(seed=0):
    rng=random.Random(seed)
    tasks=build_task_catalog()
    initial=list(tasks)[:10]
    expansion=list(tasks)[10:]
    eps=[]
    # Phase A: repetitions over stable daily tasks.
    for _ in range(60):
        k=rng.choice(initial)
        eps.append(Episode(k,tasks[k],'warmup'))
    # Phase B: more task variety and recombination.
    pool=initial+expansion
    for _ in range(80):
        k=rng.choice(pool)
        eps.append(Episode(k,tasks[k],'expansion'))
    # Phase C: same task names drift slightly; tests local repair.
    drift_targets=['news.ai','report.daily','system.health','api.weather','repo.sync']
    drifted={}
    for k in drift_targets:
        base=list(tasks[k])
        # Insert state.write just before final notify unless already there; represents evolved execution context.
        idx=max(0,len(base)-1)
        base.insert(idx,'state.write')
        drifted[k]=tuple(base)
    for _ in range(35):
        k=rng.choice(pool)
        seq=drifted.get(k,tasks[k]) if rng.random()<0.55 else tasks[k]
        eps.append(Episode(k,seq,'drift'))
    # Phase D: force old tasks out of the bounded active set, then revive them.
    # First touch many expansion tasks to evict older concepts.
    for k in expansion:
        eps.append(Episode(k,tasks[k],'evict'))
    revive=['news.ai','news.nba','report.daily','files.archive','system.health']
    for k in revive:
        eps.append(Episode(k,tasks[k],'historical_revival', True))
        eps.append(Episode(k,tasks[k],'post_integration_repeat'))
    return eps


def run_once(seed=0, emit_trace=False, outdir=None):
    eps=build_workload(seed)
    systems=[StatelessPlanner(),ExactReplay(),FlatTemplate(),DRM()]
    traces=[]
    for i,ep in enumerate(eps,1):
        for s in systems:
            r=s.step(ep)
            traces.append({'episode':i,'seed':seed,'phase':ep.phase,'task':ep.task,'system':s.name,
                           'primitive_len':len(ep.seq),**r})
    if emit_trace and outdir:
        os.makedirs(outdir,exist_ok=True)
        with open(os.path.join(outdir,f'trace_seed_{seed}.csv'),'w',newline='') as f:
            w=csv.DictWriter(f,fieldnames=sorted({k for row in traces for k in row}))
            w.writeheader(); w.writerows(traces)
    return traces,systems[-1]


def summarize(rows):
    groups=defaultdict(list)
    for r in rows: groups[r['system']].append(r)
    out=[]
    for system,rs in groups.items():
        out.append({
            'system':system,
            'episodes':len(rs),
            'mean_semantic':statistics.mean(r['semantic'] for r in rs),
            'median_semantic':statistics.median(r['semantic'] for r in rs),
            'p95_semantic':sorted(r['semantic'] for r in rs)[max(0,math.ceil(.95*len(rs))-1)],
            'total_semantic':sum(r['semantic'] for r in rs),
            'final_runtime_storage':rs[-1]['storage'],
            'final_active':rs[-1]['active'],
            'final_derived':rs[-1]['derived'],
            'recoveries':sum(r.get('recovery',0) for r in rs),
            'local_repairs':sum(r.get('local_repair',0) for r in rs),
            'structural_changes':sum(r.get('change',0) for r in rs),
        })
    return out


def aggregate_seeds(n=100):
    per=defaultdict(lambda:defaultdict(list))
    phase=defaultdict(lambda:defaultdict(list))
    derived=[]; uniform=[]; recoveries=[]; runtime=[]; history=[]
    for seed in range(n):
        rows,drm=run_once(seed)
        s=summarize(rows)
        for row in s:
            for k,v in row.items():
                if k not in ('system','episodes') and isinstance(v,(int,float)):
                    per[row['system']][k].append(v)
        for r in rows:
            phase[(r['system'],r['phase'])]['semantic'].append(r['semantic'])
        drs=[r for r in rows if r['system']=='drm_hierarchical']
        derived.append(drs[-1]['derived']); uniform.append(min(r.get('uniform',1) for r in drs))
        recoveries.append(sum(r.get('recovery',0) for r in drs)); runtime.append(drs[-1]['storage']); history.append(drs[-1].get('history_refs',0))
    def ci(vals):
        m=statistics.mean(vals); sd=statistics.stdev(vals) if len(vals)>1 else 0
        se=sd/math.sqrt(len(vals)); return m, m-1.96*se, m+1.96*se
    summary=[]
    for sys,metrics in per.items():
        row={'system':sys}
        for k,vals in metrics.items():
            m,lo,hi=ci(vals); row[k]=m; row[k+'_ci_lo']=lo; row[k+'_ci_hi']=hi
        summary.append(row)
    phase_rows=[]
    for (sys,ph),m in phase.items():
        vals=m['semantic']; phase_rows.append({'system':sys,'phase':ph,'mean_semantic':statistics.mean(vals)})
    return summary,phase_rows


def tests():
    # 1 canonical uniformity and recursive reduction.
    d=DRM(active_cap=2)
    seq=('process.spawn','browser.navigate','text.extract','text.summarize','notify.send')
    d.subseq_users[seq].update({'a','b'})
    d.maybe_add_derived()
    assert d.audit_uniformity()
    assert all(x in CANON for name in d.derived for x in d.expand_symbol(name))
    # 2 historical recovery is one-time after forward integration.
    old_seq=('fs.read','fs.move','network.check','state.read','notify.send')
    e1=Episode('old',old_seq,'t')
    d.step(e1)
    d.step(Episode('x',('fs.read','notify.send'),'t'))
    d.step(Episode('y',('http.get','notify.send'),'t')) # evicts old with cap 2
    r=d.step(Episode('old',old_seq,'t',True))
    assert r['recovery']==1
    r2=d.step(e1)
    assert r2['recovery']==0 and r2['semantic']==1
    # 3 vocabulary remains canonical-rooted over full benchmark.
    rows,drm=run_once(3)
    assert drm.audit_uniformity() and drm.uniformity_failures==0
    # 4 derived vocabulary exists and compresses at least one active representation.
    assert len(drm.derived)>0
    assert any(len(rep)<len(canon) for canon,rep in drm.active.values())
    # 5 DRM uses fewer semantic decisions than stateless on benchmark.
    sm={x['system']:x for x in summarize(rows)}
    assert sm['drm_hierarchical']['total_semantic'] < sm['stateless_planner']['total_semantic']
    return True

if __name__=='__main__':
    assert tests()
    rows,drm=run_once(0,True,'/mnt/data/drm_rust_eval/results')
    sm=summarize(rows)
    agg,ph=aggregate_seeds(30)
    with open('/mnt/data/drm_rust_eval/results/summary_seed0.csv','w',newline='') as f:
        w=csv.DictWriter(f,fieldnames=sm[0].keys());w.writeheader();w.writerows(sm)
    with open('/mnt/data/drm_rust_eval/results/aggregate_30_seeds.csv','w',newline='') as f:
        keys=sorted({k for r in agg for k in r});w=csv.DictWriter(f,fieldnames=keys);w.writeheader();w.writerows(agg)
    with open('/mnt/data/drm_rust_eval/results/phase_30_seeds.csv','w',newline='') as f:
        w=csv.DictWriter(f,fieldnames=ph[0].keys());w.writeheader();w.writerows(ph)
    print('TESTS: PASS')
    print('seed0 summary')
    for r in sm: print(r)
    print('\n30-seed aggregate')
    for r in agg:
        print(r['system'], 'mean_semantic', round(r['mean_semantic'],3),
              'total',round(r['total_semantic'],1),'storage',round(r['final_runtime_storage'],1),
              'derived',round(r['final_derived'],1),'recoveries',round(r['recoveries'],1))
    print('\nDRM derived vocab:')
    for name,dv in drm.derived.items():
        print(name,'=',dv.definition,'=>',drm.expand_symbol(name))
    print('uniform',drm.audit_uniformity(),'history_refs',drm.history_refs,'structural_changes',drm.structural_changes)

# --- Additional scenario benchmark helpers (import and call; not part of main block above) ---
def build_long_tail_workload(seed=0):
    rng=random.Random(seed)
    sources=[
        ('web',('process.spawn','browser.navigate','text.extract')),
        ('api',('network.check','http.get','text.extract')),
        ('file',('fs.read','text.extract')),
        ('proc',('process.spawn','process.wait','state.read')),
        ('archive',('fs.read','fs.move','state.read')),
    ]
    sinks=[
        ('summary',('text.summarize','notify.send')),
        ('persist',('fs.write','state.write','notify.send')),
        ('summary_persist',('text.summarize','fs.write','state.write','notify.send')),
        ('state',('state.write','notify.send')),
    ]
    episodes=[]; idx=0
    for sname,src in sources:
        for kname,sink in sinks:
            for scheduled in [False,True]:
                for context_read in [False,True]:
                    idx+=1
                    seq=((('schedule.trigger',) if scheduled else tuple()) + src +
                         (('state.read',) if context_read else tuple()) + sink)
                    task=f'longtail.{idx:03d}.{sname}.{kname}.{int(scheduled)}.{int(context_read)}'
                    episodes.append(Episode(task,seq,'novel_composition'))
    rng.shuffle(episodes)
    # Once acquired, repeat a random subset to measure post-consolidation cost.
    repeat=rng.sample(episodes,30)
    episodes += [Episode(e.task,e.seq,'consolidated_repeat') for e in repeat]
    return episodes


def run_workload(eps):
    systems=[StatelessPlanner(),ExactReplay(),FlatTemplate(),DRM()]
    rows=[]
    for i,ep in enumerate(eps,1):
        for s in systems:
            r=s.step(ep)
            rows.append({'episode':i,'phase':ep.phase,'task':ep.task,'system':s.name,
                         'primitive_len':len(ep.seq),**r})
    return rows,systems[-1]


def aggregate_long_tail(n=20):
    vals=defaultdict(lambda:defaultdict(list))
    phasevals=defaultdict(list)
    final_vocab=[]
    for seed in range(n):
        rows,drm=run_workload(build_long_tail_workload(seed))
        for r in summarize(rows):
            for k,v in r.items():
                if k not in ('system','episodes') and isinstance(v,(int,float)):
                    vals[r['system']][k].append(v)
        for r in rows:
            phasevals[(r['system'],r['phase'])].append(r['semantic'])
        final_vocab.append((len(drm.derived),drm.audit_uniformity()))
    out=[]
    for sys,m in vals.items():
        out.append({'system':sys, **{k:statistics.mean(v) for k,v in m.items()}})
    ph=[{'system':s,'phase':p,'mean_semantic':statistics.mean(v)} for (s,p),v in phasevals.items()]
    return out,ph,final_vocab
