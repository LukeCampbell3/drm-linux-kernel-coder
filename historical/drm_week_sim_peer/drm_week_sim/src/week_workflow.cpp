#define main drm_base_embedded_main
#include "base_main.cpp"
#undef main

#include <limits>
#include <numeric>
#include <optional>

namespace sd {

struct PStat {std::set<std::string> tasks;size_t hits{};size_t first_ep{};size_t last_ep{};};
struct PWord {std::string name;Seq raw;std::set<std::string> birth_tasks;size_t birth_struct{};size_t last_transfer_struct{};size_t transfer_hits{};};

struct HybridPlanner : DrmPlanner {
    std::map<Seq,PStat> pstats;
    std::map<std::string,PWord> provisional;
    size_t struct_step{};
    size_t pcounter{};
    size_t provisional_cap{20};
    size_t grace{12};
    size_t admitted_total{};
    size_t expired_total{};
    std::set<Seq> pending_touched;
    std::set<Seq> pending_touched_p;
    bool pending_consolidation{false};

    static bool contains_seq(const Seq&h,const Seq&n){if(n.empty()||n.size()>h.size())return false;for(size_t i=0;i+n.size()<=h.size();++i)if(std::equal(n.begin(),n.end(),h.begin()+static_cast<long>(i)))return true;return false;}
    std::vector<std::pair<std::string,Seq>> pexp() const {std::vector<std::pair<std::string,Seq>> x;for(const auto&[n,p]:provisional)x.push_back({n,p.raw});return x;}
    Seq compress_effective(const Seq&s) const {return vocab.compress_with(s,pexp());}
    size_t semantic_cost(const Seq&s) const {
        if(s.empty())return 0;
        std::vector<Seq> patterns;
        for(const auto&[_,raw]:vocab.expansions())if(raw.size()>=2)patterns.push_back(raw);
        for(const auto&[_,p]:provisional)if(p.raw.size()>=2)patterns.push_back(p.raw);
        std::vector<size_t> dp(s.size()+1,std::numeric_limits<size_t>::max()/4);dp[s.size()]=0;
        for(size_t pos=s.size();pos-->0;){
            dp[pos]=1+dp[pos+1];
            for(const auto&pat:patterns){
                if(pos+pat.size()>s.size())continue;
                if(std::equal(pat.begin(),pat.end(),s.begin()+static_cast<long>(pos)))dp[pos]=std::min(dp[pos],size_t(1)+dp[pos+pat.size()]);
            }
        }
        return dp[0];
    }
    bool represented(const Seq&s) const {for(const auto&[n,_]:vocab.derived)if(vocab.expand_symbol(n)==s)return true;for(const auto&[_,p]:provisional)if(p.raw==s)return true;return false;}
    std::set<Seq> note_p(const std::string&task,const Seq&s,size_t ep){std::set<Seq> touched;size_t mx=std::min<size_t>(5,s.size());for(size_t n=2;n<=mx;++n)for(size_t i=0;i+n<=s.size();++i){Seq c(s.begin()+static_cast<long>(i),s.begin()+static_cast<long>(i+n));auto&st=pstats[c];if(!st.hits)st.first_ep=ep;st.last_ep=ep;st.hits++;st.tasks.insert(task);touched.insert(c);}return touched;}
    long pscore(const Seq&c,const PStat&st) const {
        if(represented(c)||compress_effective(c).size()<=1)return std::numeric_limits<long>::min();
        if(c.size()==2&&st.tasks.size()<3)return std::numeric_limits<long>::min();
        if(c.size()>=3&&st.tasks.size()<2)return std::numeric_limits<long>::min();
        Seq def=vocab.compress(c);if(def.size()<=1)return std::numeric_limits<long>::min();
        long saving=static_cast<long>(st.tasks.size())*(static_cast<long>(c.size())-1)-static_cast<long>(def.size()+1);
        if(saving<0)return std::numeric_limits<long>::min();
        return saving*32+static_cast<long>(st.tasks.size())*8+static_cast<long>(c.size())*4;
    }
    size_t admit(const std::set<Seq>&touched){
        if(provisional.size()>=provisional_cap) return 0;
        long bs=std::numeric_limits<long>::min(); Seq best;
        for(const auto&c:touched){auto it=pstats.find(c);if(it==pstats.end())continue;long s=pscore(c,it->second);if(s>bs){bs=s;best=c;}}
        if(best.empty()) return 0;
        ++pcounter; char b[20]; std::snprintf(b,sizeof(b),"p%03zu",pcounter); auto&st=pstats.at(best); provisional.emplace(b,PWord{b,best,st.tasks,struct_step,struct_step,0}); admitted_total++; return 1;
    }
    size_t update_transfer(const Episode&ep){size_t n=0;for(auto&[_,p]:provisional){if(!contains_seq(ep.ops,p.raw)||p.birth_tasks.contains(ep.task))continue;p.transfer_hits++;p.last_transfer_struct=struct_step;n++;}return n;}
    size_t expire(){std::vector<std::string>d;for(const auto&[n,p]:provisional)if(struct_step>=p.last_transfer_struct+grace)d.push_back(n);for(const auto&n:d)provisional.erase(n);expired_total+=d.size();return d.size();}
    void remove_committed_equivalents(){std::vector<std::string>d;for(const auto&[n,p]:provisional){for(const auto&[dn,_]:vocab.derived)if(vocab.expand_symbol(dn)==p.raw){d.push_back(n);break;}}for(const auto&n:d)provisional.erase(n);}

    // Exact localized MDL: a candidate can only change compression cost for tasks
    // that have contained that subsequence. This is algebraically equivalent to
    // rescoring the whole corpus but avoids O(candidates * whole_history) scans.
    size_t maybe_grow_localized(const std::set<Seq>& candidates){
        if(candidates.empty()) return 0;
        std::set<Seq> existing; for(const auto&[k,_]:vocab.derived) existing.insert(vocab.expand_symbol(k));
        bool found=false; std::tuple<long,size_t,size_t> best_key{}; Seq best_def;
        for(const auto& cand:candidates){
            auto uit=subseq_users.find(cand); if(uit==subseq_users.end()) continue;
            const auto& userset=uit->second; if(userset.size()<2 || existing.contains(cand)) continue;
            Seq def=vocab.compress(cand); if(def.size()<=1) continue;
            long saving=0;
            for(const auto& task:userset){
                auto hit=history.find(task); if(hit==history.end()) continue;
                auto before=vocab.compress(hit->second).size();
                auto after=vocab.compress_with(hit->second,{{"__new__",cand}}).size();
                saving += static_cast<long>(before)-static_cast<long>(after);
            }
            long gain=saving-static_cast<long>(def.size()+1);
            if(gain<mdl_threshold) continue;
            auto key=std::make_tuple(gain,userset.size(),cand.size());
            if(!found || key>best_key){found=true;best_key=key;best_def=def;}
        }
        if(found){++vocab.counter;char buf[16];std::snprintf(buf,sizeof(buf),"d%03zu",vocab.counter);vocab.derived[buf]=best_def;return 1;}
        return 0;
    }

    size_t consolidate_pending(){
        if(!pending_consolidation) return 0;
        size_t changes=maybe_grow_localized(pending_touched);
        remove_committed_equivalents();
        changes += admit(pending_touched_p);
        changes += expire();
        pending_touched.clear(); pending_touched_p.clear(); pending_consolidation=false;
        return changes;
    }

    PlanMetrics plan(const Episode&ep){
        PlanMetrics m;auto ai=active.find(ep.task);auto hi=history.find(ep.task);
        if(ai!=active.end()){
            if(ai->second==ep.ops)m.semantic=1;else{auto d=diff_middle(ai->second,ep.ops);m.semantic=std::max<size_t>(1,semantic_cost(d));m.local_repair=1;m.structural_change++;}
        }else if(hi!=history.end()){
            if(ep.ancestral){m.recovery=1;m.semantic=std::max<size_t>(1,semantic_cost(ep.ops));m.structural_change++;}
            else if(hi->second!=ep.ops){auto d=diff_middle(hi->second,ep.ops);m.semantic=std::max<size_t>(1,semantic_cost(d));m.local_repair=1;m.structural_change++;}
            else m.semantic=1;
        }else {m.semantic=std::max<size_t>(1,semantic_cost(ep.ops));m.structural_change++;}
        const bool ne=(hi==history.end())||(hi->second!=ep.ops);version++;history[ep.task]=ep.ops;history_version[ep.task]=version;
        if(ne){struct_step++;update_transfer(ep);pending_touched_p=note_p(ep.task,ep.ops,ep.idx);pending_touched=note_subseqs(ep.task,ep.ops);pending_consolidation=true;}
        touch(ep.task,ep.ops);m.derived=vocab.derived.size();m.active=active.size();m.uniform=vocab.audit();m.structure_bytes=structure_bytes();
        if(!vocab.derived.empty()){size_t sum=0;for(const auto&[k,_]:vocab.derived){auto d=vocab.depth(k);sum+=d;m.max_depth=std::max(m.max_depth,d);}m.avg_depth=double(sum)/vocab.derived.size();}
        return m;
    }
};

struct StageDef {int id;std::string name;};
struct StagedEpisode {Episode ep;int stage;bool expected_repeat{false};bool post_recovery{false};};

static std::vector<Seq> motifs(){
    return {
      seq({"fs.read","transform.extract","transform.summarize","fs.write","notify.send"}),
      seq({"http.request","transform.extract","transform.summarize","state.write","notify.send"}),
      seq({"process.run","transform.extract","transform.summarize","fs.write"}),
      seq({"state.read","transform.extract","transform.summarize","fs.write","notify.send"}),
      seq({"fs.read","transform.summarize","ipc.request","state.write","notify.send"}),
      seq({"proc.observe","transform.extract","transform.summarize","ipc.request","fs.write"}),
      seq({"timer.observe","state.read","transform.summarize","fs.write","notify.send"}),
      seq({"http.request","transform.extract","ipc.request","transform.summarize","fs.write"}),
      seq({"process.run","transform.summarize","ipc.request","fs.write","notify.send"}),
      seq({"fs.read","transform.extract","ipc.request","transform.summarize","state.write"}),
      seq({"proc.observe","transform.summarize","state.write","notify.send"}),
      seq({"timer.observe","state.read","transform.extract","transform.summarize","state.write"})
    };
}

static void sadd(std::vector<StagedEpisode>&v,size_t&idx,int stage,std::string task,Seq ops,
                 std::string src="inputs/report_0.csv",std::string out="",std::string url="/news_0.html",
                 bool anc=false,bool repeat=false,bool post=false){
    ++idx;if(out.empty())out="outputs/"+task+".txt";Episode e{idx,std::move(task),"stage_"+std::to_string(stage),std::move(ops),std::move(src),std::move(out),std::move(url),anc};v.push_back({std::move(e),stage,repeat,post});
}

static std::vector<StagedEpisode> staged_workload(){
    std::vector<StagedEpisode> v;size_t i=0;auto m=motifs();
    Seq file=seq({"fs.read","transform.summarize","fs.write","notify.send"});
    Seq state=seq({"state.read","transform.summarize","state.write","notify.send"});
    Seq proc=seq({"proc.observe","transform.extract","transform.summarize","fs.write"});
    Seq hash=seq({"process.run","transform.summarize","fs.write","notify.send"});
    Seq http=seq({"http.request","transform.extract","transform.summarize","fs.write","notify.send"});
    Seq ipc=seq({"fs.read","transform.summarize","ipc.request","fs.write"});
    Seq timer=seq({"timer.observe","state.read","transform.summarize","state.write"});

    // Stage 0: canary - minimal real Linux capabilities and repeated correctness.
    for(int r=0;r<8;++r){
        sadd(v,i,0,"canary_file",file,"inputs/report_"+std::to_string(r%4)+".csv","","/news_0.html",false,r>0);
        sadd(v,i,0,"canary_state",state,"inputs/report_0.csv","","/news_0.html",false,r>0);
        sadd(v,i,0,"canary_proc",proc,"inputs/report_0.csv","","/news_0.html",false,r>0);
    }

    // Stage 1: single-user alpha - common repetitive daily routines.
    std::vector<std::pair<std::string,Seq>> daily={{"daily_file",file},{"daily_hash",hash},{"daily_http",http},{"daily_state",state},{"daily_ipc",ipc},{"daily_proc",proc},{"daily_timer",timer}};
    for(int r=0;r<12;++r){for(size_t j=0;j<daily.size();++j){auto [name,ops]=daily[j];sadd(v,i,1,name,ops,"inputs/report_"+std::to_string((r+j)%16)+".csv","","/news_"+std::to_string((r+j)%8)+".html",false,r>0);}}

    // Stage 2: connected alpha - new exact tasks sharing known motifs, then replay half.
    std::vector<Episode> stage2_first;
    for(int n=0;n<48;++n){std::string t=std::string("connected_")+(n<10?"0":"")+std::to_string(n);size_t before=v.size();sadd(v,i,2,t,m[n%m.size()],"inputs/report_"+std::to_string(n%16)+".csv","","/news_"+std::to_string(n%8)+".html");stage2_first.push_back(v[before].ep);}
    for(int n=0;n<24;++n){auto e=stage2_first[n];sadd(v,i,2,e.task,e.ops,e.source,e.output,e.url_path,false,true);}

    // Stage 3: compositional beta - 96 never-seen task identities with recombined motifs + 48 repeats.
    std::vector<Episode> stage3_first;
    for(int n=0;n<96;++n){
        Seq q=m[(n*5+n/7)%m.size()];
        // Small legal recombination every third task to create fresh exact structures while preserving familiar subgraphs.
        if(n%3==0 && std::find(q.begin(),q.end(),"transform.extract")==q.end()) q.insert(q.begin()+1,"transform.extract");
        if(n%5==0 && q.back()!="notify.send") q.push_back("notify.send");
        std::string t=std::string("beta_")+(n<10?"00":n<100?"0":"")+std::to_string(n);size_t before=v.size();sadd(v,i,3,t,q,"inputs/report_"+std::to_string((n+5)%16)+".csv","","/news_"+std::to_string((n+3)%8)+".html");stage3_first.push_back(v[before].ep);
    }
    for(int n=0;n<48;++n){auto e=stage3_first[n];sadd(v,i,3,e.task,e.ops,e.source,e.output,e.url_path,false,true);}

    // Stage 4: resilience beta - drift, eviction pressure, historical recovery, immediate forward consolidation.
    sadd(v,i,4,"daily_file",m[0],"inputs/report_13.csv");
    sadd(v,i,4,"daily_http",m[1],"inputs/report_0.csv","","/news_7.html");
    sadd(v,i,4,"daily_hash",m[8],"inputs/report_14.csv");
    sadd(v,i,4,"daily_ipc",m[9],"inputs/report_15.csv");
    for(int n=0;n<48;++n)sadd(v,i,4,"evict_"+std::to_string(n),m[(n+2)%m.size()],"inputs/report_"+std::to_string((n+3)%16)+".csv","","/news_"+std::to_string(n%8)+".html");
    for(const std::string t:{"daily_http","daily_file","daily_hash","daily_ipc"}){
        auto it=std::find_if(v.begin(),v.end(),[&](const StagedEpisode&se){return se.stage==1&&se.ep.task==t;});
        sadd(v,i,4,t,it->ep.ops,it->ep.source,it->ep.output,it->ep.url_path,true,false,false);
        sadd(v,i,4,t,it->ep.ops,it->ep.source,it->ep.output,it->ep.url_path,false,true,true);
    }

    // Stage 5: release-candidate sustained mixed use. Mostly mature routines with periodic never-seen compositions.
    for(int cycle=0;cycle<60;++cycle){
        // Five high-frequency mature routines each cycle.
        for(size_t j=0;j<5;++j){auto [name,ops]=daily[j];sadd(v,i,5,name,ops,"inputs/report_"+std::to_string((cycle+j)%16)+".csv","","/news_"+std::to_string((cycle+j)%8)+".html",false,true);}
        // Every fifth cycle introduce a new exact task from the known structural family.
        if(cycle%5==0){int n=cycle/5;Seq q=m[(n*7+3)%m.size()];std::string t="rc_novel_"+std::to_string(n);sadd(v,i,5,t,q,"inputs/report_"+std::to_string((n+9)%16)+".csv","","/news_"+std::to_string((n+1)%8)+".html");}
        // Every tenth cycle exercise timer/proc routines too.
        if(cycle%10==0){sadd(v,i,5,"daily_proc",proc,"inputs/report_0.csv","","/news_0.html",false,true);sadd(v,i,5,"daily_timer",timer,"inputs/report_0.csv","","/news_0.html",false,true);}
    }
    return v;
}

static std::string stage_name(int s){switch(s){case 0:return"canary_core";case 1:return"single_user_alpha";case 2:return"connected_alpha";case 3:return"compositional_beta";case 4:return"resilience_beta";case 5:return"release_candidate";default:return"unknown";}}
static double percentile(std::vector<double> v,double q){if(v.empty())return 0;std::sort(v.begin(),v.end());double p=(v.size()-1)*q;size_t lo=static_cast<size_t>(std::floor(p)),hi=static_cast<size_t>(std::ceil(p));if(lo==hi)return v[lo];double f=p-lo;return v[lo]*(1-f)+v[hi]*f;}

struct StageMetric{
    int stage{};size_t episodes{},success{},semantic{},structural{},recoveries{},repairs{},first_seen{},first_semantic{},first_one{},repeat_count{},repeat_semantic{},post_recovery{},post_recovery_zero{};
    double planner_sum{},consolidation_sum{},executor_sum{},wall_sum{},cpu_u{},cpu_s{};std::vector<double> planner,consolidation,executor,wall;long peak_rss{};long long read_b{},write_b{};size_t derived_start{},derived_end{},prov_start{},prov_end{},struct_bytes_end{},raw_tokens{},compressed_tokens{},def_tokens{},dense_bytes{},fused_bytes{},unique_blocks{};bool uniform{true};
};

static void snapshot(StageMetric&sm,const HybridPlanner&p){
    sm.derived_end=p.vocab.derived.size();sm.prov_end=p.provisional.size();sm.struct_bytes_end=p.structure_bytes();
    std::set<Seq> uniq;sm.raw_tokens=sm.compressed_tokens=sm.def_tokens=0;
    for(const auto&[_,s]:p.history){sm.raw_tokens+=s.size();sm.compressed_tokens+=p.vocab.compress(s).size();uniq.insert(s);}
    for(const auto&[_,d]:p.vocab.derived)sm.def_tokens+=d.size();
    sm.dense_bytes=sm.raw_tokens;sm.unique_blocks=uniq.size();size_t ub=0;for(const auto&s:uniq)ub+=s.size();sm.fused_bytes=ub+2*p.history.size();
}

[[maybe_unused]] static int run(const fs::path& out){
    fs::remove_all(out);fs::create_directories(out);auto work=out/"workspace";make_fixtures(work);TcpServer tcp;UnixServer uds(work/"drm.sock");LiveExecutor ex(work,tcp.port,work/"drm.sock");HybridPlanner drm;drm.provisional_cap=20;auto eps=staged_workload();
    std::array<StageMetric,6> metrics;for(int s=0;s<6;++s){metrics[s].stage=s;metrics[s].derived_start=drm.vocab.derived.size();metrics[s].prov_start=drm.provisional.size();}
    Baseline stateless;stateless.kind="stateless";Baseline cache;cache.kind="template_cache";Baseline checkpoint;checkpoint.kind="checkpoint_replay";
    std::array<size_t,6> bstat{},bcache{},bcheck{};
    std::set<std::string> seen_tasks;
    std::ofstream tr(out/"staged_trace.csv");
    tr<<"episode,stage,stage_name,task,success,first_seen,expected_repeat,post_recovery,wall_ms,planner_ms,consolidation_ms,executor_ms,cpu_user_ms,cpu_sys_ms,rss_kb,read_bytes,write_bytes,semantic,recovery,repair,structural_change,permanent_words,provisional_words,structure_bytes,uniform\n";
    auto all0=Clock::now();size_t total_ok=0,total_sem=0;
    for(const auto&se:eps){auto&e=se.ep;auto&s=metrics[se.stage];s.episodes++;bool first=!seen_tasks.contains(e.task);if(first){seen_tasks.insert(e.task);s.first_seen++;}
        auto u0=usage();auto io0=io();auto t0=Clock::now();auto p0=Clock::now();auto pm=drm.plan(e);auto p1=Clock::now();std::string err;auto x0=Clock::now();bool ok=ex.execute(e,err);auto x1=Clock::now();auto c0=Clock::now();size_t post_changes=drm.consolidate_pending();auto c1=Clock::now();pm.structural_change+=post_changes;auto t1=Clock::now();auto u1=usage();auto io1=io();
        double wall=std::chrono::duration<double,std::milli>(t1-t0).count(), plan=std::chrono::duration<double,std::milli>(p1-p0).count(), consolidation=std::chrono::duration<double,std::milli>(c1-c0).count(), exec=std::chrono::duration<double,std::milli>(x1-x0).count();long rss=rss_kb();
        s.success+=ok;s.semantic+=pm.semantic;s.structural+=pm.structural_change;s.recoveries+=pm.recovery;s.repairs+=pm.local_repair;s.planner_sum+=plan;s.consolidation_sum+=consolidation;s.executor_sum+=exec;s.wall_sum+=wall;s.cpu_u+=(u1.u-u0.u)*1000;s.cpu_s+=(u1.s-u0.s)*1000;s.planner.push_back(plan);s.consolidation.push_back(consolidation);s.executor.push_back(exec);s.wall.push_back(wall);s.peak_rss=std::max(s.peak_rss,rss);s.read_b+=io1.read_bytes-io0.read_bytes;s.write_b+=io1.write_bytes-io0.write_bytes;s.uniform=s.uniform&&pm.uniform;
        if(first){s.first_semantic+=pm.semantic;if(pm.semantic==1)s.first_one++;}if(se.expected_repeat){s.repeat_count++;s.repeat_semantic+=pm.semantic;}if(se.post_recovery){s.post_recovery++;if(pm.recovery==0)s.post_recovery_zero++;}
        total_ok+=ok;total_sem+=pm.semantic;
        bstat[se.stage]+=stateless.plan(e).semantic;bcache[se.stage]+=cache.plan(e).semantic;bcheck[se.stage]+=checkpoint.plan(e).semantic;
        tr<<e.idx<<','<<se.stage<<','<<stage_name(se.stage)<<','<<esc(e.task)<<','<<ok<<','<<first<<','<<se.expected_repeat<<','<<se.post_recovery<<','<<std::fixed<<std::setprecision(6)<<wall<<','<<plan<<','<<consolidation<<','<<exec<<','<<(u1.u-u0.u)*1000<<','<<(u1.s-u0.s)*1000<<','<<rss<<','<<(io1.read_bytes-io0.read_bytes)<<','<<(io1.write_bytes-io0.write_bytes)<<','<<pm.semantic<<','<<pm.recovery<<','<<pm.local_repair<<','<<pm.structural_change<<','<<drm.vocab.derived.size()<<','<<drm.provisional.size()<<','<<pm.structure_bytes<<','<<pm.uniform<<"\n";
        snapshot(s,drm);
        // capture the real start state for the next stage at boundary
        if(e.idx<eps.size() && eps[e.idx].stage!=se.stage){auto&n=metrics[eps[e.idx].stage];n.derived_start=drm.vocab.derived.size();n.prov_start=drm.provisional.size();}
    }
    double total_ms=std::chrono::duration<double,std::milli>(Clock::now()-all0).count();

    std::ofstream sc(out/"stage_metrics.csv");
    sc<<"stage,stage_name,episodes,success_rate,semantic_total,semantic_mean,first_seen,first_seen_semantic_mean,first_seen_one_decision_rate,repeats,repeat_semantic_mean,structural_events,recoveries,repairs,post_recovery_zero_rate,permanent_start,permanent_end,provisional_start,provisional_end,planner_mean_ms,planner_p95_ms,consolidation_mean_ms,consolidation_p95_ms,executor_mean_ms,wall_mean_ms,cpu_user_ms,cpu_sys_ms,peak_rss_kb,read_bytes,write_bytes,structure_bytes,raw_tokens,compressed_tokens,definition_tokens,description_reduction,dense_microcode_bytes,fused_microcode_bytes,unique_blocks,stateless_semantic,template_semantic,checkpoint_semantic,uniform\n";
    for(auto&s:metrics){double desc=s.raw_tokens?1.0-double(s.compressed_tokens+s.def_tokens+s.derived_end)/s.raw_tokens:0;sc<<s.stage<<','<<stage_name(s.stage)<<','<<s.episodes<<','<<double(s.success)/s.episodes<<','<<s.semantic<<','<<double(s.semantic)/s.episodes<<','<<s.first_seen<<','<<(s.first_seen?double(s.first_semantic)/s.first_seen:0)<<','<<(s.first_seen?double(s.first_one)/s.first_seen:0)<<','<<s.repeat_count<<','<<(s.repeat_count?double(s.repeat_semantic)/s.repeat_count:0)<<','<<s.structural<<','<<s.recoveries<<','<<s.repairs<<','<<(s.post_recovery?double(s.post_recovery_zero)/s.post_recovery:1)<<','<<s.derived_start<<','<<s.derived_end<<','<<s.prov_start<<','<<s.prov_end<<','<<s.planner_sum/s.episodes<<','<<percentile(s.planner,.95)<<','<<s.consolidation_sum/s.episodes<<','<<percentile(s.consolidation,.95)<<','<<s.executor_sum/s.episodes<<','<<s.wall_sum/s.episodes<<','<<s.cpu_u<<','<<s.cpu_s<<','<<s.peak_rss<<','<<s.read_b<<','<<s.write_b<<','<<s.struct_bytes_end<<','<<s.raw_tokens<<','<<s.compressed_tokens<<','<<s.def_tokens<<','<<desc<<','<<s.dense_bytes<<','<<s.fused_bytes<<','<<s.unique_blocks<<','<<bstat[s.stage]<<','<<bcache[s.stage]<<','<<bcheck[s.stage]<<','<<s.uniform<<"\n";}

    // Vocabulary/provisional audit at final state.
    std::ofstream va(out/"final_vocabulary.csv");va<<"kind,name,length,transfer_hits,depth,root_uniform,definition\n";auto join=[](const Seq&s){std::string o;for(size_t i=0;i<s.size();++i){if(i)o+=" > ";o+=s[i];}return o;};
    for(const auto&[n,d]:drm.vocab.derived){auto roots=drm.vocab.expand_root(n);bool uni=std::all_of(roots.begin(),roots.end(),is_root);va<<"permanent,"<<n<<','<<drm.vocab.expand_symbol(n).size()<<",0,"<<drm.vocab.depth(n)<<','<<uni<<','<<esc(join(d))<<"\n";}
    for(const auto&[n,p]:drm.provisional){bool uni=true;for(const auto&cap:p.raw){auto it=CAP_ROOT.find(cap);if(it==CAP_ROOT.end())uni=false;else for(const auto&r:it->second)if(!is_root(r))uni=false;}va<<"provisional,"<<n<<','<<p.raw.size()<<','<<p.transfer_hits<<",0,"<<uni<<','<<esc(join(p.raw))<<"\n";}

    // Deployment gates derived from observed metrics, not claimed as universal thresholds.
    std::ofstream gates(out/"deployment_gates.csv");gates<<"stage,stage_name,success_100,uniform_vocab,planner_p95_under_10ms,post_recovery_consolidates,repeat_mean_at_most_1_1,gate_pass\n";
    bool allg=true;for(auto&s:metrics){bool g1=s.success==s.episodes,g2=s.uniform,g3=percentile(s.planner,.95)<10.0,g4=(s.post_recovery==0||s.post_recovery_zero==s.post_recovery),g5=(s.repeat_count==0||double(s.repeat_semantic)/s.repeat_count<=1.1);bool gp=g1&&g2&&g3&&g4&&g5;allg&=gp;gates<<s.stage<<','<<stage_name(s.stage)<<','<<g1<<','<<g2<<','<<g3<<','<<g4<<','<<g5<<','<<gp<<"\n";}

    auto&last=metrics.back();double final_desc=last.raw_tokens?1.0-double(last.compressed_tokens+last.def_tokens+last.derived_end)/last.raw_tokens:0;
    std::ofstream js(out/"session_summary.json");js<<std::fixed<<std::setprecision(6)<<"{\n"<<"  \"episodes\": "<<eps.size()<<",\n"<<"  \"success_rate\": "<<double(total_ok)/eps.size()<<",\n"<<"  \"semantic_total\": "<<total_sem<<",\n"<<"  \"semantic_mean\": "<<double(total_sem)/eps.size()<<",\n"<<"  \"permanent_words_final\": "<<drm.vocab.derived.size()<<",\n"<<"  \"provisional_words_final\": "<<drm.provisional.size()<<",\n"<<"  \"provisional_admitted_total\": "<<drm.admitted_total<<",\n"<<"  \"provisional_expired_total\": "<<drm.expired_total<<",\n"<<"  \"uniform_vocabulary\": "<<(drm.vocab.audit()?"true":"false")<<",\n"<<"  \"description_length_reduction\": "<<final_desc<<",\n"<<"  \"dense_microcode_bytes\": "<<last.dense_bytes<<",\n"<<"  \"fused_microcode_bytes\": "<<last.fused_bytes<<",\n"<<"  \"string_structure_bytes\": "<<last.struct_bytes_end<<",\n"<<"  \"peak_rss_kb\": "<<std::max_element(metrics.begin(),metrics.end(),[](const StageMetric&a,const StageMetric&b){return a.peak_rss<b.peak_rss;})->peak_rss<<",\n"<<"  \"benchmark_wall_ms\": "<<total_ms<<",\n"<<"  \"process_spawns\": "<<ex.process_spawns<<",\n"<<"  \"tcp_requests\": "<<ex.tcp_requests<<",\n"<<"  \"ipc_requests\": "<<ex.ipc_requests<<",\n"<<"  \"timer_events\": "<<ex.timer_events<<",\n"<<"  \"commits\": "<<ex.commits<<",\n"<<"  \"deployment_gates_all_pass\": "<<(allg?"true":"false")<<",\n"<<"  \"root_counts\": {\"OBSERVE\": "<<ex.root_counts["OBSERVE"]<<", \"DERIVE\": "<<ex.root_counts["DERIVE"]<<", \"COMMIT\": "<<ex.root_counts["COMMIT"]<<"}\n"<<"}\n";
    std::cout<<"episodes="<<eps.size()<<" success="<<total_ok<<" semantic="<<total_sem<<" permanent="<<drm.vocab.derived.size()<<" provisional="<<drm.provisional.size()<<" fused_bytes="<<last.fused_bytes<<" gates="<<allg<<" wall_ms="<<total_ms<<"\n";
    return total_ok==eps.size()&&drm.vocab.audit()&&allg?0:2;
}

} // namespace sd


namespace week {
using sd::HybridPlanner;

struct WeekEpisode {
    Episode ep;
    int day{};
    std::string day_name;
    std::string category;
    double manual_minutes{}; // scenario assumption, not measured machine time
    bool scheduled{true};
    bool post_recovery{false};
};

static void wadd(std::vector<WeekEpisode>& v, size_t& idx, int day, const std::string& day_name,
                 std::string task, Seq ops, std::string category, double manual_minutes,
                 std::string src="inputs/report_0.csv", std::string out="", std::string url="/news_0.html",
                 bool ancestral=false, bool scheduled=true, bool post_recovery=false) {
    ++idx;
    if(out.empty()) out="outputs/week_"+std::to_string(day)+"_"+task+".txt";
    Episode e{idx,std::move(task),"week_day_"+std::to_string(day),std::move(ops),std::move(src),std::move(out),std::move(url),ancestral};
    v.push_back({std::move(e),day,day_name,std::move(category),manual_minutes,scheduled,post_recovery});
}

static std::vector<WeekEpisode> weekly_workload(){
    std::vector<WeekEpisode> v; size_t i=0; auto m=sd::motifs();
    const std::array<std::string,7> dn={"Mon Aug 17","Tue Aug 18","Wed Aug 19","Thu Aug 20","Fri Aug 21","Sat Aug 22","Sun Aug 23"};
    Seq morning=seq({"http.request","transform.extract","transform.summarize","state.write","notify.send"});
    Seq reports=seq({"fs.read","transform.extract","transform.summarize","fs.write","notify.send"});
    Seq health=seq({"proc.observe","transform.extract","transform.summarize","state.write","notify.send"});
    Seq backup=seq({"process.run","transform.summarize","fs.write","notify.send"});
    Seq sync=seq({"fs.read","transform.summarize","ipc.request","state.write","notify.send"});
    Seq reminders=seq({"timer.observe","state.read","transform.summarize","state.write","notify.send"});
    Seq state=seq({"state.read","transform.summarize","state.write","notify.send"});
    Seq digest=seq({"fs.read","transform.extract","transform.summarize","ipc.request","fs.write","notify.send"});

    // Monday: cold start. Repetitive routines plus first project-specific work.
    wadd(v,i,0,dn[0],"morning_digest",morning,"information",12.0,"inputs/report_0.csv","","/news_0.html");
    wadd(v,i,0,dn[0],"report_intake",reports,"office",8.0,"inputs/report_1.csv");
    wadd(v,i,0,dn[0],"system_health",health,"system",4.0);
    wadd(v,i,0,dn[0],"backup_verify",backup,"system",3.0,"inputs/report_2.csv");
    wadd(v,i,0,dn[0],"local_sync",sync,"office",3.0,"inputs/report_3.csv");
    wadd(v,i,0,dn[0],"reminder_refresh",reminders,"admin",2.0);
    wadd(v,i,0,dn[0],"state_snapshot",state,"admin",2.0);
    wadd(v,i,0,dn[0],"project_alpha",m[2],"project",9.0,"inputs/report_4.csv");
    wadd(v,i,0,dn[0],"project_beta",m[7],"project",10.0,"inputs/report_5.csv","","/news_1.html");
    wadd(v,i,0,dn[0],"project_gamma",m[9],"project",7.0,"inputs/report_6.csv");
    wadd(v,i,0,dn[0],"afternoon_digest",morning,"information",8.0,"inputs/report_0.csv","","/news_2.html");
    wadd(v,i,0,dn[0],"closeout",digest,"office",6.0,"inputs/report_7.csv");

    // Tuesday: same user rhythm, plus related new project variants.
    for(int r=0;r<2;++r){
        wadd(v,i,1,dn[1],"morning_digest",morning,"information",12.0,"inputs/report_0.csv","","/news_3.html");
        wadd(v,i,1,dn[1],"report_intake",reports,"office",8.0,"inputs/report_8.csv");
        wadd(v,i,1,dn[1],"system_health",health,"system",4.0);
        wadd(v,i,1,dn[1],"local_sync",sync,"office",3.0,"inputs/report_9.csv");
    }
    wadd(v,i,1,dn[1],"project_delta",m[3],"project",8.0,"inputs/report_10.csv");
    wadd(v,i,1,dn[1],"project_epsilon",m[10],"project",6.0,"inputs/report_11.csv");
    wadd(v,i,1,dn[1],"backup_verify",backup,"system",3.0,"inputs/report_12.csv");
    wadd(v,i,1,dn[1],"reminder_refresh",reminders,"admin",2.0);
    wadd(v,i,1,dn[1],"closeout",digest,"office",6.0,"inputs/report_13.csv");

    // Wednesday: broader compositional work; task names are new but structures share learned motifs.
    wadd(v,i,2,dn[2],"morning_digest",morning,"information",12.0,"inputs/report_0.csv","","/news_4.html");
    wadd(v,i,2,dn[2],"report_intake",reports,"office",8.0,"inputs/report_14.csv");
    wadd(v,i,2,dn[2],"system_health",health,"system",4.0);
    for(int n=0;n<8;++n){
        Seq q=m[(n*5+1)%m.size()];
        if(n%3==0 && q.back()!="notify.send") q.push_back("notify.send");
        wadd(v,i,2,dn[2],"client_job_"+std::to_string(n),q,"project",5.0+(n%4)*1.5,
             "inputs/report_"+std::to_string((n+1)%16)+".csv","","/news_"+std::to_string((n+5)%8)+".html");
    }
    wadd(v,i,2,dn[2],"local_sync",sync,"office",3.0,"inputs/report_15.csv");
    wadd(v,i,2,dn[2],"reminder_refresh",reminders,"admin",2.0);
    wadd(v,i,2,dn[2],"closeout",digest,"office",6.0,"inputs/report_2.csv");

    // Thursday: normal routines plus two controlled drifts (new source/extra IPC leg).
    Seq morning_drift=seq({"http.request","transform.extract","ipc.request","transform.summarize","state.write","notify.send"});
    Seq reports_drift=seq({"fs.read","transform.extract","transform.summarize","ipc.request","fs.write","notify.send"});
    wadd(v,i,3,dn[3],"morning_digest",morning_drift,"information",12.0,"inputs/report_0.csv","","/news_6.html");
    wadd(v,i,3,dn[3],"report_intake",reports_drift,"office",8.0,"inputs/report_3.csv");
    wadd(v,i,3,dn[3],"system_health",health,"system",4.0);
    wadd(v,i,3,dn[3],"backup_verify",backup,"system",3.0,"inputs/report_4.csv");
    wadd(v,i,3,dn[3],"local_sync",sync,"office",3.0,"inputs/report_5.csv");
    for(int n=0;n<10;++n) wadd(v,i,3,dn[3],"ad_hoc_"+std::to_string(n),m[(n+4)%m.size()],"project",4.0+(n%3)*2.0,
                               "inputs/report_"+std::to_string((n+6)%16)+".csv","","/news_"+std::to_string(n%8)+".html");
    wadd(v,i,3,dn[3],"closeout",digest,"office",6.0,"inputs/report_6.csv");

    // Friday: mature work plus explicit ancestral recovery of Monday's old project_alpha context.
    wadd(v,i,4,dn[4],"morning_digest",morning_drift,"information",12.0,"inputs/report_0.csv","","/news_7.html");
    wadd(v,i,4,dn[4],"report_intake",reports_drift,"office",8.0,"inputs/report_7.csv");
    wadd(v,i,4,dn[4],"system_health",health,"system",4.0);
    wadd(v,i,4,dn[4],"project_alpha",m[2],"project",9.0,"inputs/report_4.csv","","/news_0.html",true,true,false);
    wadd(v,i,4,dn[4],"project_alpha",m[2],"project",9.0,"inputs/report_4.csv","","/news_0.html",false,true,true);
    for(int n=0;n<8;++n) wadd(v,i,4,dn[4],"friday_job_"+std::to_string(n),m[(n*7+2)%m.size()],"project",5.0+(n%4),
                              "inputs/report_"+std::to_string((n+8)%16)+".csv","","/news_"+std::to_string((n+2)%8)+".html");
    wadd(v,i,4,dn[4],"weekly_work_summary",digest,"office",15.0,"inputs/report_8.csv");
    wadd(v,i,4,dn[4],"backup_verify",backup,"system",3.0,"inputs/report_9.csv");

    // Weekend: low load but autonomous maintenance continues.
    wadd(v,i,5,dn[5],"morning_digest",morning_drift,"information",8.0,"inputs/report_0.csv","","/news_1.html");
    wadd(v,i,5,dn[5],"system_health",health,"system",3.0);
    wadd(v,i,5,dn[5],"backup_verify",backup,"system",3.0,"inputs/report_10.csv");
    wadd(v,i,5,dn[5],"weekend_cleanup",m[4],"admin",6.0,"inputs/report_11.csv");
    wadd(v,i,5,dn[5],"reminder_refresh",reminders,"admin",2.0);

    wadd(v,i,6,dn[6],"morning_digest",morning_drift,"information",8.0,"inputs/report_0.csv","","/news_2.html");
    wadd(v,i,6,dn[6],"system_health",health,"system",3.0);
    wadd(v,i,6,dn[6],"backup_verify",backup,"system",3.0,"inputs/report_12.csv");
    wadd(v,i,6,dn[6],"weekly_personal_summary",digest,"admin",10.0,"inputs/report_13.csv");
    wadd(v,i,6,dn[6],"next_week_prepare",m[11],"admin",8.0,"inputs/report_14.csv");
    wadd(v,i,6,dn[6],"reminder_refresh",reminders,"admin",2.0);
    return v;
}

struct DayMetric {
    int day{}; std::string name; size_t episodes{},success{},semantic{},first_seen{},first_semantic{},first_one{},structural{},recoveries{},repairs{};
    double plan_sum{},cons_sum{},exec_sum{},wall_sum{},cpu_u{},cpu_s{},manual_min{},attention_sec{};
    std::vector<double> planner,wall; long peak_rss{}; long long read_b{},write_b{}; size_t permanent_start{},permanent_end{},prov_start{},prov_end{},structure_bytes{}; bool uniform{true};
};

static double attention_seconds(bool first,const PlanMetrics& pm,bool scheduled){
    // Explicit scenario model: first-seen work gets a 30 s review, familiar scheduled
    // work a 4 s glance, and structural repair/recovery gets an additional 20 s.
    // This is NOT machine benchmark time; it is used only to estimate human attention value.
    double s = first ? 30.0 : (scheduled ? 4.0 : 8.0);
    if(pm.semantic>1) s += 3.0*static_cast<double>(pm.semantic-1);
    if(pm.local_repair) s += 20.0;
    if(pm.recovery) s += 20.0;
    return s;
}

static int run_week(const fs::path& out){
    fs::remove_all(out); fs::create_directories(out); auto work=out/"workspace"; make_fixtures(work);
    TcpServer tcp; UnixServer uds(work/"drm.sock"); LiveExecutor ex(work,tcp.port,work/"drm.sock");
    HybridPlanner drm; drm.provisional_cap=20; auto eps=weekly_workload();
    Baseline stateless; stateless.kind="stateless"; Baseline cache; cache.kind="template_cache"; Baseline checkpoint; checkpoint.kind="checkpoint_replay";
    std::array<DayMetric,7> dm; for(int d=0;d<7;++d){dm[d].day=d; dm[d].permanent_start=drm.vocab.derived.size(); dm[d].prov_start=drm.provisional.size();}
    std::array<size_t,7> bstat{},bcache{},bcheck{}; std::set<std::string> seen;
    std::ofstream tr(out/"week_trace.csv");
    tr<<"episode,day,day_name,task,category,manual_minutes,success,first_seen,semantic,recovery,repair,structural_change,planner_ms,consolidation_ms,executor_ms,wall_ms,attention_seconds,permanent_words,provisional_words,structure_bytes,rss_kb,read_bytes,write_bytes,uniform\n";
    auto all0=Clock::now(); size_t ok_total=0,sem_total=0; double manual_total=0,attn_total=0;
    for(const auto& we:eps){
        auto& e=we.ep; auto& d=dm[we.day]; if(d.name.empty()) d.name=we.day_name; d.episodes++; bool first=!seen.contains(e.task); if(first){seen.insert(e.task);d.first_seen++;}
        auto u0=usage(); auto io0=io(); auto t0=Clock::now(); auto p0=Clock::now(); auto pm=drm.plan(e); auto p1=Clock::now(); std::string err; auto x0=Clock::now(); bool ok=ex.execute(e,err); auto x1=Clock::now(); auto c0=Clock::now(); size_t pc=drm.consolidate_pending(); auto c1=Clock::now(); pm.structural_change+=pc; auto t1=Clock::now(); auto u1=usage(); auto io1=io();
        double plan=std::chrono::duration<double,std::milli>(p1-p0).count(); double cons=std::chrono::duration<double,std::milli>(c1-c0).count(); double exec=std::chrono::duration<double,std::milli>(x1-x0).count(); double wall=std::chrono::duration<double,std::milli>(t1-t0).count(); long rss=rss_kb(); double attn=attention_seconds(first,pm,we.scheduled);
        d.success+=ok; d.semantic+=pm.semantic; d.structural+=pm.structural_change; d.recoveries+=pm.recovery; d.repairs+=pm.local_repair; d.plan_sum+=plan; d.cons_sum+=cons; d.exec_sum+=exec; d.wall_sum+=wall; d.cpu_u+=(u1.u-u0.u)*1000; d.cpu_s+=(u1.s-u0.s)*1000; d.manual_min+=we.manual_minutes; d.attention_sec+=attn; d.planner.push_back(plan); d.wall.push_back(wall); d.peak_rss=std::max(d.peak_rss,rss); d.read_b+=io1.read_bytes-io0.read_bytes; d.write_b+=io1.write_bytes-io0.write_bytes; d.uniform=d.uniform&&pm.uniform; if(first){d.first_semantic+=pm.semantic;if(pm.semantic==1)d.first_one++;}
        ok_total+=ok; sem_total+=pm.semantic; manual_total+=we.manual_minutes; attn_total+=attn;
        bstat[we.day]+=stateless.plan(e).semantic; bcache[we.day]+=cache.plan(e).semantic; bcheck[we.day]+=checkpoint.plan(e).semantic;
        d.permanent_end=drm.vocab.derived.size();d.prov_end=drm.provisional.size();d.structure_bytes=drm.structure_bytes();
        tr<<e.idx<<','<<we.day<<','<<esc(we.day_name)<<','<<esc(e.task)<<','<<we.category<<','<<we.manual_minutes<<','<<ok<<','<<first<<','<<pm.semantic<<','<<pm.recovery<<','<<pm.local_repair<<','<<pm.structural_change<<','<<std::fixed<<std::setprecision(6)<<plan<<','<<cons<<','<<exec<<','<<wall<<','<<attn<<','<<drm.vocab.derived.size()<<','<<drm.provisional.size()<<','<<pm.structure_bytes<<','<<rss<<','<<(io1.read_bytes-io0.read_bytes)<<','<<(io1.write_bytes-io0.write_bytes)<<','<<pm.uniform<<"\n";
        if(e.idx<eps.size() && eps[e.idx].day!=we.day){auto& nd=dm[eps[e.idx].day];nd.permanent_start=drm.vocab.derived.size();nd.prov_start=drm.provisional.size();}
    }
    double total_wall=std::chrono::duration<double,std::milli>(Clock::now()-all0).count();
    std::ofstream dc(out/"day_metrics.csv");
    dc<<"day,day_name,episodes,success_rate,semantic_total,semantic_mean,first_seen,first_seen_semantic_mean,first_seen_one_decision_rate,structural_events,recoveries,repairs,planner_mean_ms,planner_p95_ms,consolidation_mean_ms,executor_mean_ms,wall_mean_ms,peak_rss_kb,manual_minutes_modeled,human_attention_minutes_modeled,human_minutes_returned_modeled,attention_leverage,permanent_start,permanent_end,provisional_start,provisional_end,structure_bytes,stateless_semantic,template_semantic,checkpoint_semantic,uniform\n";
    for(auto& d:dm){double attm=d.attention_sec/60.0;double returned=std::max(0.0,d.manual_min-attm);dc<<d.day<<','<<esc(d.name)<<','<<d.episodes<<','<<double(d.success)/d.episodes<<','<<d.semantic<<','<<double(d.semantic)/d.episodes<<','<<d.first_seen<<','<<(d.first_seen?double(d.first_semantic)/d.first_seen:0)<<','<<(d.first_seen?double(d.first_one)/d.first_seen:0)<<','<<d.structural<<','<<d.recoveries<<','<<d.repairs<<','<<d.plan_sum/d.episodes<<','<<sd::percentile(d.planner,.95)<<','<<d.cons_sum/d.episodes<<','<<d.exec_sum/d.episodes<<','<<d.wall_sum/d.episodes<<','<<d.peak_rss<<','<<d.manual_min<<','<<attm<<','<<returned<<','<<(attm>0?d.manual_min/attm:0)<<','<<d.permanent_start<<','<<d.permanent_end<<','<<d.prov_start<<','<<d.prov_end<<','<<d.structure_bytes<<','<<bstat[d.day]<<','<<bcache[d.day]<<','<<bcheck[d.day]<<','<<d.uniform<<"\n";}
    size_t raw=0,compressed=0,defs=0;std::set<Seq> uniq;for(const auto&[_,s]:drm.history){raw+=s.size();compressed+=drm.vocab.compress(s).size();uniq.insert(s);}for(const auto&[_,d]:drm.vocab.derived)defs+=d.size();size_t fused=2*drm.history.size();for(const auto&s:uniq)fused+=s.size();double desc=raw?1.0-double(compressed+defs+drm.vocab.derived.size())/raw:0;
    double attention_min=attn_total/60.0,returned=std::max(0.0,manual_total-attention_min);
    size_t stat_total=std::accumulate(bstat.begin(),bstat.end(),size_t(0)),cache_total=std::accumulate(bcache.begin(),bcache.end(),size_t(0)),check_total=std::accumulate(bcheck.begin(),bcheck.end(),size_t(0));
    std::ofstream js(out/"week_summary.json");js<<std::fixed<<std::setprecision(6)<<"{\n"<<"  \"episodes\": "<<eps.size()<<",\n"<<"  \"success_rate\": "<<double(ok_total)/eps.size()<<",\n"<<"  \"semantic_total\": "<<sem_total<<",\n"<<"  \"semantic_mean\": "<<double(sem_total)/eps.size()<<",\n"<<"  \"stateless_semantic\": "<<stat_total<<",\n"<<"  \"template_semantic\": "<<cache_total<<",\n"<<"  \"checkpoint_semantic\": "<<check_total<<",\n"<<"  \"permanent_words_final\": "<<drm.vocab.derived.size()<<",\n"<<"  \"provisional_words_final\": "<<drm.provisional.size()<<",\n"<<"  \"provisional_admitted_total\": "<<drm.admitted_total<<",\n"<<"  \"provisional_expired_total\": "<<drm.expired_total<<",\n"<<"  \"description_length_reduction\": "<<desc<<",\n"<<"  \"raw_microops\": "<<raw<<",\n"<<"  \"fused_microcode_bytes\": "<<fused<<",\n"<<"  \"unique_workflow_blocks\": "<<uniq.size()<<",\n"<<"  \"string_structure_bytes\": "<<drm.structure_bytes()<<",\n"<<"  \"benchmark_wall_ms\": "<<total_wall<<",\n"<<"  \"modeled_manual_minutes\": "<<manual_total<<",\n"<<"  \"modeled_attention_minutes\": "<<attention_min<<",\n"<<"  \"modeled_minutes_returned\": "<<returned<<",\n"<<"  \"modeled_attention_leverage\": "<<(attention_min>0?manual_total/attention_min:0)<<",\n"<<"  \"process_spawns\": "<<ex.process_spawns<<",\n"<<"  \"tcp_requests\": "<<ex.tcp_requests<<",\n"<<"  \"ipc_requests\": "<<ex.ipc_requests<<",\n"<<"  \"timer_events\": "<<ex.timer_events<<",\n"<<"  \"commits\": "<<ex.commits<<",\n"<<"  \"uniform_vocabulary\": "<<(drm.vocab.audit()?"true":"false")<<",\n"<<"  \"root_counts\": {\"OBSERVE\": "<<ex.root_counts["OBSERVE"]<<", \"DERIVE\": "<<ex.root_counts["DERIVE"]<<", \"COMMIT\": "<<ex.root_counts["COMMIT"]<<"}\n"<<"}\n";
    std::cout<<"week episodes="<<eps.size()<<" success="<<ok_total<<" semantic="<<sem_total<<" permanent="<<drm.vocab.derived.size()<<" provisional="<<drm.provisional.size()<<" manual_min="<<manual_total<<" attention_min="<<attention_min<<" returned_min="<<returned<<" wall_ms="<<total_wall<<"\n";
    return ok_total==eps.size()&&drm.vocab.audit()?0:2;
}

} // namespace week

int main(int argc,char**argv){try{fs::path out="results";for(int i=1;i+1<argc;++i)if(std::string(argv[i])=="--out")out=argv[i+1];return week::run_week(out);}catch(const std::exception&e){std::cerr<<"fatal: "<<e.what()<<"\n";return 1;}}
