#define main drm_base_embedded_main
#include "base_main.cpp"
#undef main

#include <limits>

namespace hy {

struct CascadePlanner : DrmPlanner {
    size_t growth_cascade_cap{4};
    PlanMetrics plan(const Episode& ep){
        PlanMetrics m; auto ai=active.find(ep.task); auto hi=history.find(ep.task);
        if(ai!=active.end()){
            if(ai->second==ep.ops)m.semantic=1; else {auto d=diff_middle(ai->second,ep.ops);m.semantic=std::max<size_t>(1,vocab.compress(d).size());m.local_repair=1;m.structural_change++;}
        } else if(hi!=history.end()){
            if(ep.ancestral){m.recovery=1;m.semantic=std::max<size_t>(1,vocab.compress(ep.ops).size());m.structural_change++;}
            else if(hi->second!=ep.ops){auto d=diff_middle(hi->second,ep.ops);m.semantic=std::max<size_t>(1,vocab.compress(d).size());m.local_repair=1;m.structural_change++;}
            else m.semantic=1;
        } else {m.semantic=std::max<size_t>(1,vocab.compress(ep.ops).size());m.structural_change++;}
        const bool new_evidence=(hi==history.end())||(hi->second!=ep.ops);
        version++;history[ep.task]=ep.ops;history_version[ep.task]=version;
        if(new_evidence){
            note_subseqs(ep.task,ep.ops);
            std::set<Seq> all;for(const auto&[cand,_]:subseq_users)all.insert(cand);
            for(size_t k=0;k<growth_cascade_cap;++k){size_t g=maybe_grow(all);if(!g)break;m.structural_change+=g;}
        }
        touch(ep.task,ep.ops);
        m.derived=vocab.derived.size();m.active=active.size();m.uniform=vocab.audit();m.structure_bytes=structure_bytes();
        if(!vocab.derived.empty()){size_t sum=0;for(const auto&[k,_]:vocab.derived){auto d=vocab.depth(k);sum+=d;m.max_depth=std::max(m.max_depth,d);}m.avg_depth=double(sum)/vocab.derived.size();}
        return m;
    }
};

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
    size_t admit(const std::set<Seq>&touched){if(provisional.size()>=provisional_cap)return 0;long bs=std::numeric_limits<long>::min();Seq best;for(const auto&c:touched){auto it=pstats.find(c);if(it==pstats.end())continue;long s=pscore(c,it->second);if(s>bs){bs=s;best=c;}}if(best.empty())return 0;++pcounter;char b[20];std::snprintf(b,sizeof(b),"p%03zu",pcounter);auto&st=pstats.at(best);provisional.emplace(b,PWord{b,best,st.tasks,struct_step,struct_step,0});admitted_total++;return 1;}
    size_t update_transfer(const Episode&ep){size_t n=0;for(auto&[_,p]:provisional){if(!contains_seq(ep.ops,p.raw)||p.birth_tasks.contains(ep.task))continue;p.transfer_hits++;p.last_transfer_struct=struct_step;n++;}return n;}
    size_t expire(){std::vector<std::string>d;for(const auto&[n,p]:provisional)if(struct_step>=p.last_transfer_struct+grace)d.push_back(n);for(const auto&n:d)provisional.erase(n);expired_total+=d.size();return d.size();}
    void remove_committed_equivalents(){std::vector<std::string>d;for(const auto&[n,p]:provisional){for(const auto&[dn,_]:vocab.derived)if(vocab.expand_symbol(dn)==p.raw){d.push_back(n);break;}}for(const auto&n:d)provisional.erase(n);}

    PlanMetrics plan(const Episode&ep){
        PlanMetrics m;auto ai=active.find(ep.task);auto hi=history.find(ep.task);
        if(ai!=active.end()){
            if(ai->second==ep.ops)m.semantic=1;else{auto d=diff_middle(ai->second,ep.ops);m.semantic=std::max<size_t>(1,semantic_cost(d));m.local_repair=1;m.structural_change++;}
        }else if(hi!=history.end()){
            if(ep.ancestral){m.recovery=1;m.semantic=std::max<size_t>(1,semantic_cost(ep.ops));m.structural_change++;}
            else if(hi->second!=ep.ops){auto d=diff_middle(hi->second,ep.ops);m.semantic=std::max<size_t>(1,semantic_cost(d));m.local_repair=1;m.structural_change++;}
            else m.semantic=1;
        }else m.semantic=std::max<size_t>(1,semantic_cost(ep.ops)),m.structural_change++;
        const bool ne=(hi==history.end())||(hi->second!=ep.ops);version++;history[ep.task]=ep.ops;history_version[ep.task]=version;
        if(ne){
            struct_step++;update_transfer(ep);auto touched_p=note_p(ep.task,ep.ops,ep.idx);
            auto touched=note_subseqs(ep.task,ep.ops);size_t grew=maybe_grow(touched);
            remove_committed_equivalents();size_t a=admit(touched_p);size_t x=expire();m.structural_change+=grew+a+x;
        }
        touch(ep.task,ep.ops);m.derived=vocab.derived.size();m.active=active.size();m.uniform=vocab.audit();m.structure_bytes=structure_bytes();return m;
    }
};

struct Summary{std::string name;size_t semantic{},first_any{},first_commit{},last_commit{},last_struct{},words{},provisional{},recoveries{},repairs{},admitted{},expired{},compressed{},raw{},bytes{};double ms{};bool uniform{};};
static size_t rawtok(const std::map<std::string,Seq>&h){size_t n=0;for(const auto&[_,s]:h)n+=s.size();return n;}
static size_t comptok(const Vocabulary&v,const std::map<std::string,Seq>&h){size_t n=0;for(const auto&[_,s]:h)n+=v.compress(s).size();return n;}

template<class P> Summary execute(std::string name,P&p,const std::vector<Episode>&eps,const fs::path&trace){Summary r;r.name=name;size_t prev=0,prevany=0;std::ofstream tr(trace);tr<<"episode,task,phase,semantic,words,provisional,structural\n";auto t0=Clock::now();for(const auto&e:eps){auto m=p.plan(e);r.semantic+=m.semantic;r.recoveries+=m.recovery;r.repairs+=m.local_repair;if(m.structural_change)r.last_struct=e.idx;size_t pv=0;if constexpr(std::is_same_v<P,HybridPlanner>)pv=p.provisional.size();size_t any=p.vocab.derived.size()+pv;if(any!=prevany){if(!r.first_any)r.first_any=e.idx;prevany=any;}if(p.vocab.derived.size()!=prev){if(!r.first_commit)r.first_commit=e.idx;r.last_commit=e.idx;prev=p.vocab.derived.size();}tr<<e.idx<<','<<e.task<<','<<e.phase<<','<<m.semantic<<','<<p.vocab.derived.size()<<','<<pv<<','<<m.structural_change<<"\n";}r.ms=std::chrono::duration<double,std::milli>(Clock::now()-t0).count();r.words=p.vocab.derived.size();if constexpr(std::is_same_v<P,HybridPlanner>){r.provisional=p.provisional.size();r.admitted=p.admitted_total;r.expired=p.expired_total;}r.raw=rawtok(p.history);r.compressed=comptok(p.vocab,p.history);r.bytes=p.structure_bytes();r.uniform=p.vocab.audit();return r;}

static bool self_test(){if(!::self_test())return false;HybridPlanner h;auto eps=workload();for(size_t i=0;i<30;++i)h.plan(eps[i]);return h.vocab.audit()&&!h.provisional.empty();}

static int run(const fs::path&out){
    fs::remove_all(out);fs::create_directories(out);auto eps=workload();
    DrmPlanner b;HybridPlanner h;
    auto sb=execute("baseline",b,eps,out/"baseline_trace.csv");
    auto sh=execute("hybrid",h,eps,out/"hybrid_trace.csv");
    std::ofstream f(out/"comparison.csv");
    f<<"system,semantic,first_any,first_committed,last_committed,last_struct,words,provisional,admitted,expired,learn_ms,structure_bytes,raw_tokens,compressed_tokens,compression,uniform\n";
    auto emit=[&](const Summary&s){f<<s.name<<','<<s.semantic<<','<<s.first_any<<','<<s.first_commit<<','<<s.last_commit<<','<<s.last_struct<<','<<s.words<<','<<s.provisional<<','<<s.admitted<<','<<s.expired<<','<<s.ms<<','<<s.bytes<<','<<s.raw<<','<<s.compressed<<','<<(1.0-double(s.compressed)/s.raw)<<','<<s.uniform<<"\n";};
    emit(sb);emit(sh);
    std::ofstream vf(out/"hybrid_vocabulary.csv");vf<<"kind,name,length,sequence\n";
    for(const auto&[name,_]:h.vocab.derived){auto raw=h.vocab.expand_symbol(name);vf<<"committed,"<<name<<','<<raw.size()<<',';for(size_t i=0;i<raw.size();++i){if(i)vf<<'|';vf<<raw[i];}vf<<"\n";}
    for(const auto&[name,p]:h.provisional){vf<<"provisional,"<<name<<','<<p.raw.size()<<',';for(size_t i=0;i<p.raw.size();++i){if(i)vf<<'|';vf<<p.raw[i];}vf<<"\n";}
    std::ofstream j(out/"summary.json");
    j<<std::fixed<<std::setprecision(6)<<"{\n"
      <<"  \"baseline_semantic\": "<<sb.semantic<<",\n"
      <<"  \"hybrid_semantic\": "<<sh.semantic<<",\n"
      <<"  \"hybrid_semantic_reduction\": "<<(1.0-double(sh.semantic)/sb.semantic)<<",\n"
      <<"  \"baseline_first_word\": "<<sb.first_commit<<",\n"
      <<"  \"hybrid_first_effective_word\": "<<sh.first_any<<",\n"
      <<"  \"baseline_last_word\": "<<sb.last_commit<<",\n"
      <<"  \"hybrid_final_committed\": "<<sh.words<<",\n"
      <<"  \"hybrid_final_provisional\": "<<sh.provisional<<",\n"
      <<"  \"hybrid_admitted_total\": "<<sh.admitted<<",\n"
      <<"  \"hybrid_expired_total\": "<<sh.expired<<",\n"
      <<"  \"uniform\": "<<(sh.uniform?"true":"false")<<"\n}\n";
    std::cout<<"base="<<sb.semantic<<" hybrid="<<sh.semantic<<" first_effective="<<sh.first_any<<" base_last="<<sb.last_commit<<" hybrid_words="<<sh.words<<" p="<<sh.provisional<<" ms="<<sh.ms<<"\n";
    return (sb.uniform&&sh.uniform&&sh.semantic<=sb.semantic)?0:2;
}


} // namespace hy

int main(int argc,char**argv){try{if(argc>1&&std::string(argv[1])=="--self-test"){bool ok=hy::self_test();std::cout<<(ok?"HYBRID_SELF_TEST_PASS":"HYBRID_SELF_TEST_FAIL")<<"\n";return ok?0:1;}fs::path out="results";for(int i=1;i+1<argc;++i)if(std::string(argv[i])=="--out")out=argv[i+1];return hy::run(out);}catch(const std::exception&e){std::cerr<<"fatal: "<<e.what()<<"\n";return 1;}}
