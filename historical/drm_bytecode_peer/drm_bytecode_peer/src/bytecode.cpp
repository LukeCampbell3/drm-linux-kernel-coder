#define main drm_base_embedded_main
#include "base_main.cpp"
#undef main

#include <cstdint>
#include <limits>
#include <optional>
#include <unordered_map>

namespace bc {

enum class RootOp : uint16_t { OBS=0, DRV=1, CMT=2 };
enum class Operand : uint8_t { NONE=0, SOURCE=1, OUTPUT=2, URL=3, STATE=4, TEMP=5 };

struct CapInfo { uint8_t id; const char* name; Operand operand; };
static constexpr std::array<CapInfo,12> CAPS = {{
    {0,"fs.read",Operand::SOURCE}, {1,"state.read",Operand::STATE}, {2,"proc.observe",Operand::NONE},
    {3,"timer.observe",Operand::NONE}, {4,"http.request",Operand::URL}, {5,"ipc.request",Operand::TEMP},
    {6,"process.run",Operand::SOURCE}, {7,"transform.extract",Operand::TEMP}, {8,"transform.summarize",Operand::TEMP},
    {9,"fs.write",Operand::OUTPUT}, {10,"state.write",Operand::STATE}, {11,"notify.send",Operand::TEMP}
}};

static const CapInfo& cap_info(const std::string& name){
    for(const auto& c:CAPS) if(name==c.name) return c;
    throw std::runtime_error("unknown capability: "+name);
}
static const CapInfo& cap_info(uint8_t id){
    for(const auto& c:CAPS) if(id==c.id) return c;
    throw std::runtime_error("bad capability id");
}
static RootOp root_op(const std::string& r){
    if(r=="OBSERVE") return RootOp::OBS;
    if(r=="DERIVE") return RootOp::DRV;
    if(r=="COMMIT") return RootOp::CMT;
    throw std::runtime_error("non-root opcode: "+r);
}
static const char* root_name(RootOp r){
    switch(r){case RootOp::OBS:return "OBS";case RootOp::DRV:return "DRV";case RootOp::CMT:return "CMT";}
    return "BAD";
}

// 16-bit DRM instruction:
// [15:14] semantic opcode (00 OBS, 01 DRV, 10 CMT, 11 reserved)
// [13:8]  capability id (0..63)
// [7:0]   operand/context slot
static uint16_t pack(RootOp op,uint8_t cap,Operand operand){
    return static_cast<uint16_t>((static_cast<uint16_t>(op)<<14) | ((cap&0x3fu)<<8) | static_cast<uint8_t>(operand));
}
static RootOp op_of(uint16_t w){return static_cast<RootOp>((w>>14)&0x3u);}
static uint8_t cap_of(uint16_t w){return static_cast<uint8_t>((w>>8)&0x3fu);}
static Operand operand_of(uint16_t w){return static_cast<Operand>(w&0xffu);}
static bool valid_word(uint16_t w){return ((w>>14)&0x3u)!=3u && cap_of(w)<CAPS.size();}

struct Program { std::vector<uint16_t> words; };

struct DenseProgram { std::vector<uint8_t> words; };

// Dense 8-bit micro-op:
// [7:4] capability id (0..15)
// [3:2] frozen O/D/C signature (00 O, 01 D, 10 D-C, 11 D-C-O)
// [1:0] reserved/version flags
static uint8_t signature_of(const Seq& roots){
    if(roots==seq({"OBSERVE"})) return 0;
    if(roots==seq({"DERIVE"})) return 1;
    if(roots==seq({"DERIVE","COMMIT"})) return 2;
    if(roots==seq({"DERIVE","COMMIT","OBSERVE"})) return 3;
    throw std::runtime_error("unsupported ODC signature");
}
static Seq roots_of_signature(uint8_t s){
    switch(s&0x3u){case 0:return seq({"OBSERVE"});case 1:return seq({"DERIVE"});case 2:return seq({"DERIVE","COMMIT"});case 3:return seq({"DERIVE","COMMIT","OBSERVE"});}
    return {};
}
static uint8_t dense_pack(uint8_t cap){
    if(cap>=16) throw std::runtime_error("dense capability overflow");
    const auto& ci=cap_info(cap);uint8_t sig=signature_of(CAP_ROOT.at(ci.name));return static_cast<uint8_t>((cap<<4)|(sig<<2));
}
static uint8_t dense_cap(uint8_t w){return static_cast<uint8_t>((w>>4)&0x0fu);}
static uint8_t dense_sig(uint8_t w){return static_cast<uint8_t>((w>>2)&0x03u);}
static bool dense_valid(uint8_t w){if((w&0x03u)!=0)return false;uint8_t c=dense_cap(w);if(c>=CAPS.size())return false;return roots_of_signature(dense_sig(w))==CAP_ROOT.at(cap_info(c).name);}
static DenseProgram compile_dense(const Seq&ops){DenseProgram p;p.words.reserve(ops.size());for(const auto&cap:ops)p.words.push_back(dense_pack(cap_info(cap).id));return p;}
static Seq dense_decode_caps(const DenseProgram&p){Seq out;out.reserve(p.words.size());for(uint8_t w:p.words){if(!dense_valid(w))throw std::runtime_error("bad dense word");out.emplace_back(cap_info(dense_cap(w)).name);}return out;}
static Seq dense_decode_roots(const DenseProgram&p){Seq out;for(uint8_t w:p.words){if(!dense_valid(w))throw std::runtime_error("bad dense word");auto r=roots_of_signature(dense_sig(w));out.insert(out.end(),r.begin(),r.end());}return out;}

static Program compile_caps(const Seq& ops){
    Program p;
    for(const auto& cap:ops){
        const auto& ci=cap_info(cap);
        const auto& roots=CAP_ROOT.at(cap);
        for(const auto& r:roots) p.words.push_back(pack(root_op(r),ci.id,ci.operand));
    }
    return p;
}
static Seq decode_caps(const Program& p){
    Seq out;
    size_t i=0;
    while(i<p.words.size()){
        if(!valid_word(p.words[i])) throw std::runtime_error("invalid word");
        uint8_t cid=cap_of(p.words[i]);
        const auto& ci=cap_info(cid);
        const auto& expected=CAP_ROOT.at(ci.name);
        if(i+expected.size()>p.words.size()) throw std::runtime_error("truncated capability expansion");
        for(size_t j=0;j<expected.size();++j){
            uint16_t w=p.words[i+j];
            if(!valid_word(w)||cap_of(w)!=cid||op_of(w)!=root_op(expected[j])||operand_of(w)!=ci.operand)
                throw std::runtime_error("semantic expansion mismatch");
        }
        out.emplace_back(ci.name);
        i+=expected.size();
    }
    return out;
}
static Seq decode_roots(const Program& p){
    Seq out; out.reserve(p.words.size());
    for(uint16_t w:p.words){if(!valid_word(w))throw std::runtime_error("invalid word");auto o=op_of(w);out.emplace_back(o==RootOp::OBS?"OBSERVE":o==RootOp::DRV?"DERIVE":"COMMIT");}
    return out;
}
static std::string dump(const Program&p){
    std::ostringstream o;
    for(size_t i=0;i<p.words.size();++i){auto w=p.words[i];const auto& c=cap_info(cap_of(w));o<<i<<' '<<root_name(op_of(w))<<" c"<<int(c.id)<<" @"<<int(static_cast<uint8_t>(operand_of(w)))<<" ; "<<c.name<<'\n';}
    return o.str();
}

static std::string join_ops(const Seq&s){std::string o;for(size_t i=0;i<s.size();++i){if(i)o+='|';o+=s[i];}return o;}

static std::string join_ops(const Seq&s);

struct BlockTable {
    std::vector<Program> blocks;
    std::map<std::string,uint16_t> by_ops;
    uint16_t intern(const Seq& ops){
        std::string key=join_ops(ops);auto it=by_ops.find(key);if(it!=by_ops.end())return it->second;
        if(blocks.size()>=std::numeric_limits<uint16_t>::max())throw std::runtime_error("block table overflow");
        uint16_t id=static_cast<uint16_t>(blocks.size());blocks.push_back(compile_caps(ops));by_ops[key]=id;return id;
    }
    const Program& get(uint16_t id)const{if(id>=blocks.size())throw std::runtime_error("bad block id");return blocks[id];}
};

struct DenseBlockTable {
    std::vector<DenseProgram> blocks;std::map<std::string,uint16_t> by_ops;
    uint16_t intern(const Seq&ops){std::string key=join_ops(ops);auto it=by_ops.find(key);if(it!=by_ops.end())return it->second;if(blocks.size()>=std::numeric_limits<uint16_t>::max())throw std::runtime_error("dense block overflow");uint16_t id=static_cast<uint16_t>(blocks.size());blocks.push_back(compile_dense(ops));by_ops[key]=id;return id;}
    const DenseProgram& get(uint16_t id)const{if(id>=blocks.size())throw std::runtime_error("bad dense block id");return blocks[id];}
};

static size_t graph_bytes(const Seq& ops){size_t n=0;for(const auto&x:ops)n+=x.size()+1;return n;}
static Seq roots_for_caps(const Seq&ops){Seq r;for(const auto&c:ops){const auto&x=CAP_ROOT.at(c);r.insert(r.end(),x.begin(),x.end());}return r;}

static bool apply_cap(LiveExecutor& ex,const Episode& ep,const std::string& cap,std::string& data,std::string& err){
    try{
        if(cap=="fs.read") data=(std::ostringstream{}<<std::ifstream(ex.work/ep.source).rdbuf()).str();
        else if(cap=="state.read"){std::ifstream f(ex.work/"state.txt");data=f?(std::ostringstream{}<<f.rdbuf()).str():"runs=0";}
        else if(cap=="proc.observe"){std::ifstream f("/proc/self/status");data=(std::ostringstream{}<<f.rdbuf()).str();}
        else if(cap=="timer.observe"){auto until=Clock::now()+std::chrono::milliseconds(2);while(Clock::now()<until)std::this_thread::sleep_for(std::chrono::microseconds(200));data="timer-fired";ex.timer_events++;}
        else if(cap=="http.request")data=ex.http_get(ep.url_path);
        else if(cap=="ipc.request")data=ex.unix_roundtrip(data.empty()?ep.task:data.substr(0,std::min<size_t>(80,data.size())));
        else if(cap=="process.run")data=ex.run_hash(ex.work/ep.source);
        else if(cap=="transform.extract")data=LiveExecutor::extract(data);
        else if(cap=="transform.summarize")data=LiveExecutor::summarize(data);
        else if(cap=="fs.write"){auto out=ex.work/ep.output;fs::create_directories(out.parent_path());auto tmp=out;tmp+=".candidate";{std::ofstream f(tmp);f<<data;}fs::rename(tmp,out);ex.commits++;}
        else if(cap=="state.write"){ex.state_runs++;auto tmp=ex.work/"state.candidate";{std::ofstream f(tmp);f<<"runs="<<ex.state_runs<<" last="<<data.substr(0,120);}fs::rename(tmp,ex.work/"state.txt");ex.commits++;}
        else if(cap=="notify.send"){std::ofstream f(ex.work/"notifications.log",std::ios::app);f<<data.substr(0,300)<<"\n";ex.commits++;}
        else throw std::runtime_error("unknown cap "+cap);
        return true;
    }catch(const std::exception&e){err=e.what();return false;}
}

static bool execute_bytecode(LiveExecutor& ex,const Episode& ep,const Program&p,std::string&err){
    std::string data;
    size_t i=0;
    try{
        while(i<p.words.size()){
            uint16_t first=p.words[i];if(!valid_word(first))throw std::runtime_error("invalid bytecode word");uint8_t cid=cap_of(first);const auto& ci=cap_info(cid);const auto& roots=CAP_ROOT.at(ci.name);
            if(i+roots.size()>p.words.size())throw std::runtime_error("truncated bytecode");
            for(size_t j=0;j<roots.size();++j){uint16_t w=p.words[i+j];if(cap_of(w)!=cid||op_of(w)!=root_op(roots[j]))throw std::runtime_error("root sequence mismatch");ex.root_counts[roots[j]]++;}
            if(!apply_cap(ex,ep,ci.name,data,err))return false;
            i+=roots.size();
        }
        if(std::find(ep.ops.begin(),ep.ops.end(),"fs.write")!=ep.ops.end()){auto pth=ex.work/ep.output;if(!fs::exists(pth)||fs::file_size(pth)==0)throw std::runtime_error("verify output");}
        return true;
    }catch(const std::exception&e){err=e.what();return false;}
}

static bool execute_block(LiveExecutor& ex,const Episode&ep,const BlockTable&bt,uint16_t id,std::string&err){return execute_bytecode(ex,ep,bt.get(id),err);}

static bool execute_dense(LiveExecutor&ex,const Episode&ep,const DenseProgram&p,std::string&err){
    std::string data;
    try{
        for(uint8_t w:p.words){if(!dense_valid(w))throw std::runtime_error("invalid dense word");const auto&ci=cap_info(dense_cap(w));auto roots=roots_of_signature(dense_sig(w));for(const auto&r:roots)ex.root_counts[r]++;if(!apply_cap(ex,ep,ci.name,data,err))return false;}
        if(std::find(ep.ops.begin(),ep.ops.end(),"fs.write")!=ep.ops.end()){auto pth=ex.work/ep.output;if(!fs::exists(pth)||fs::file_size(pth)==0)throw std::runtime_error("verify output");}return true;
    }catch(const std::exception&e){err=e.what();return false;}
}
static bool execute_dense_block(LiveExecutor&ex,const Episode&ep,const DenseBlockTable&bt,uint16_t id,std::string&err){return execute_dense(ex,ep,bt.get(id),err);}

struct DispatchStats { double graph_ns{}, bytecode_ns{}, block_ns{}, dense_ns{}, dense_block_ns{}; uint64_t checksum{}; };

static DispatchStats microbench(const std::vector<Seq>& seqs,const BlockTable&bt,const std::vector<uint16_t>&ids,const DenseBlockTable&dbt,const std::vector<uint16_t>&dids){
    constexpr size_t rounds=250000;
    volatile uint64_t sink=0;
    auto t0=Clock::now();
    for(size_t r=0;r<rounds;++r){const auto&s=seqs[r%seqs.size()];for(const auto&cap:s){const auto& roots=CAP_ROOT.at(cap);for(const auto&root:roots)sink+=static_cast<unsigned char>(root[0])+cap.size();}}
    auto t1=Clock::now();
    for(size_t r=0;r<rounds;++r){const auto&p=bt.get(ids[r%ids.size()]);for(uint16_t w:p.words)sink+=static_cast<uint16_t>(op_of(w))+cap_of(w)+static_cast<uint8_t>(operand_of(w));}
    auto t2=Clock::now();
    for(size_t r=0;r<rounds;++r){uint16_t id=ids[r%ids.size()];const auto&p=bt.get(id);sink+=id;for(uint16_t w:p.words)sink+=static_cast<uint16_t>(op_of(w))+cap_of(w);}
    auto t3=Clock::now();
    for(size_t r=0;r<rounds;++r){const auto&p=dbt.get(dids[r%dids.size()]);for(uint8_t w:p.words){sink+=dense_cap(w)+dense_sig(w);}}
    auto t4=Clock::now();
    for(size_t r=0;r<rounds;++r){uint16_t id=dids[r%dids.size()];const auto&p=dbt.get(id);sink+=id;for(uint8_t w:p.words)sink+=dense_cap(w);}
    auto t5=Clock::now();
    double ops=static_cast<double>(rounds);
    return {std::chrono::duration<double,std::nano>(t1-t0).count()/ops,std::chrono::duration<double,std::nano>(t2-t1).count()/ops,std::chrono::duration<double,std::nano>(t3-t2).count()/ops,std::chrono::duration<double,std::nano>(t4-t3).count()/ops,std::chrono::duration<double,std::nano>(t5-t4).count()/ops,static_cast<uint64_t>(sink)};
}

static std::vector<Episode> unique_exec_subset(const std::vector<Episode>& all){
    std::vector<Episode> out;std::set<std::string> seen;
    for(const auto&e:all){std::string k=join_ops(e.ops);if(seen.insert(k).second){out.push_back(e);if(out.size()>=12)break;}}
    return out;
}

static bool self_test(){
    if(!::self_test())return false;
    for(uint16_t op=0;op<3;++op)for(uint8_t c=0;c<CAPS.size();++c){auto w=pack(static_cast<RootOp>(op),c,CAPS[c].operand);if(!valid_word(w)||static_cast<uint16_t>(op_of(w))!=op||cap_of(w)!=c)return false;}
    Seq s=seq({"fs.read","transform.summarize","fs.write"});auto p=compile_caps(s);if(decode_caps(p)!=s)return false;if(decode_roots(p)!=roots_for_caps(s))return false;
    if(p.words.size()!=4)return false; // OBS + DRV + DRV,CMT
    BlockTable bt;auto a=bt.intern(s);auto b=bt.intern(s);if(a!=b||bt.blocks.size()!=1)return false;
    auto dp=compile_dense(s);if(dense_decode_caps(dp)!=s||dense_decode_roots(dp)!=roots_for_caps(s))return false;for(uint8_t w:dp.words)if(!dense_valid(w))return false;
    DenseBlockTable dbt;auto da=dbt.intern(s);auto db=dbt.intern(s);if(da!=db||dbt.blocks.size()!=1)return false;
    return true;
}

static int run(const fs::path&out){
    fs::remove_all(out);fs::create_directories(out);
    auto all=workload();
    DrmPlanner planner;size_t semantic_total=0,recoveries=0,repairs=0,structural_changes=0;for(const auto&e:all){auto m=planner.plan(e);semantic_total+=m.semantic;recoveries+=m.recovery;repairs+=m.local_repair;structural_changes+=m.structural_change;}

    std::map<std::string,Seq> final_tasks=planner.history;
    BlockTable bt;DenseBlockTable dbt;
    size_t graph_total=0,byte_total=0,dense_total=0,task_refs=0;
    std::vector<Seq> unique_sequences;std::vector<uint16_t> unique_ids,dense_unique_ids;std::set<std::string> unique_keys;
    std::ofstream ra(out/"representation_audit.csv");ra<<"task,capabilities,root_ops,graph_bytes,bytecode_words,bytecode_bytes,block_id,task_ref_bytes,roundtrip,uniform\n";
    size_t roundtrip_tasks=0,root_equivalent_tasks=0,dense_roundtrip_tasks=0;
    for(const auto&[task,ops]:final_tasks){auto p=compile_caps(ops);auto dec=decode_caps(p);auto roots=decode_roots(p);bool uniform=std::all_of(roots.begin(),roots.end(),is_root);uint16_t id=bt.intern(ops);uint16_t did=dbt.intern(ops);auto dp=compile_dense(ops);roundtrip_tasks+=(dec==ops);root_equivalent_tasks+=(roots==roots_for_caps(ops));dense_roundtrip_tasks+=(dense_decode_caps(dp)==ops && dense_decode_roots(dp)==roots);size_t gb=graph_bytes(ops),bb=p.words.size()*sizeof(uint16_t);graph_total+=gb;byte_total+=bb;dense_total+=dp.words.size();task_refs+=sizeof(uint16_t);std::string key=join_ops(ops);if(unique_keys.insert(key).second){unique_sequences.push_back(ops);unique_ids.push_back(id);dense_unique_ids.push_back(did);}ra<<task<<','<<ops.size()<<','<<roots.size()<<','<<gb<<','<<p.words.size()<<','<<bb<<','<<id<<','<<sizeof(uint16_t)<<','<<(dec==ops)<<','<<uniform<<"\n";}
    size_t block_table_bytes=0;for(const auto&p:bt.blocks)block_table_bytes+=p.words.size()*sizeof(uint16_t);
    size_t fused_total=task_refs+block_table_bytes;
    size_t dense_block_table_bytes=0;for(const auto&p:dbt.blocks)dense_block_table_bytes+=p.words.size();
    size_t dense_fused_total=task_refs+dense_block_table_bytes;

    std::ofstream va(out/"derived_vocab_bytecode.csv");va<<"word,depth,definition_tokens,expanded_caps,bytecode_words,bytecode_bytes,roundtrip,uniform\n";
    bool all_uniform=true;for(const auto&[name,def]:planner.vocab.derived){auto caps=planner.vocab.expand_symbol(name);auto p=compile_caps(caps);auto roots=decode_roots(p);bool uni=std::all_of(roots.begin(),roots.end(),is_root)&&decode_caps(p)==caps;all_uniform&=uni;va<<name<<','<<planner.vocab.depth(name)<<','<<def.size()<<','<<caps.size()<<','<<p.words.size()<<','<<p.words.size()*2<<','<<(decode_caps(p)==caps)<<','<<uni<<"\n";}

    auto ds=microbench(unique_sequences,bt,unique_ids,dbt,dense_unique_ids);
    std::ofstream mb(out/"dispatch_benchmark.csv");mb<<"mode,ns_per_task_dispatch,speedup_vs_graph\n";mb<<"graph_strings,"<<ds.graph_ns<<",1\n";mb<<"odc_bytecode,"<<ds.bytecode_ns<<','<<(ds.graph_ns/ds.bytecode_ns)<<"\n";mb<<"fused_block,"<<ds.block_ns<<','<<(ds.graph_ns/ds.block_ns)<<"\n";mb<<"dense_microop,"<<ds.dense_ns<<','<<(ds.graph_ns/ds.dense_ns)<<"\n";mb<<"dense_fused_block,"<<ds.dense_block_ns<<','<<(ds.graph_ns/ds.dense_block_ns)<<"\n";

    // Real Linux equivalence on identical unique workflows, isolated workspaces.
    auto subset=unique_exec_subset(all);
    auto wg=out/"graph_ws", wb=out/"bytecode_ws", wf=out/"block_ws", wd=out/"dense_ws", wdf=out/"dense_block_ws";make_fixtures(wg);make_fixtures(wb);make_fixtures(wf);make_fixtures(wd);make_fixtures(wdf);
    TcpServer tg,tb,tf,td,tdf;UnixServer ug(wg/"drm.sock"),ub(wb/"drm.sock"),uf(wf/"drm.sock"),ud(wd/"drm.sock"),udf(wdf/"drm.sock");LiveExecutor eg(wg,tg.port,wg/"drm.sock"), eb(wb,tb.port,wb/"drm.sock"), ef(wf,tf.port,wf/"drm.sock"), ed(wd,td.port,wd/"drm.sock"), edf(wdf,tdf.port,wdf/"drm.sock");
    size_t gok=0,bok=0,fok=0,dok=0,dfok=0,equiv=0;double gms=0,bms=0,fms=0,dms=0,dfms=0;
    std::ofstream live(out/"live_equivalence.csv");live<<"task,graph_ok,bytecode_ok,block_ok,dense_ok,dense_block_ok,graph_ms,bytecode_ms,block_ms,dense_ms,dense_block_ms,uniform\n";
    for(auto e:subset){e.output="outputs/"+e.task+".txt";std::string er1,er2,er3;auto p=compile_caps(e.ops);uint16_t id=bt.intern(e.ops);
        auto dp=compile_dense(e.ops);uint16_t did=dbt.intern(e.ops);std::string er4,er5;auto a0=Clock::now();bool o1=eg.execute(e,er1);auto a1=Clock::now();bool o2=execute_bytecode(eb,e,p,er2);auto a2=Clock::now();bool o3=execute_block(ef,e,bt,id,er3);auto a3=Clock::now();bool o4=execute_dense(ed,e,dp,er4);auto a4=Clock::now();bool o5=execute_dense_block(edf,e,dbt,did,er5);auto a5=Clock::now();double x=std::chrono::duration<double,std::milli>(a1-a0).count(),y=std::chrono::duration<double,std::milli>(a2-a1).count(),z=std::chrono::duration<double,std::milli>(a3-a2).count(),q=std::chrono::duration<double,std::milli>(a4-a3).count(),v=std::chrono::duration<double,std::milli>(a5-a4).count();gms+=x;bms+=y;fms+=z;dms+=q;dfms+=v;gok+=o1;bok+=o2;fok+=o3;dok+=o4;dfok+=o5;bool uni=std::all_of(decode_roots(p).begin(),decode_roots(p).end(),is_root)&&dense_decode_roots(dp)==decode_roots(p);equiv+=(o1&&o2&&o3&&o4&&o5);live<<e.task<<','<<o1<<','<<o2<<','<<o3<<','<<o4<<','<<o5<<','<<x<<','<<y<<','<<z<<','<<q<<','<<v<<','<<uni<<"\n";}

    // Historical compatibility: old and drifted programs coexist immutably.
    Seq oldops=seq({"fs.read","transform.summarize","fs.write","notify.send"});Seq newops=seq({"fs.read","transform.extract","transform.summarize","fs.write","notify.send"});auto oldp=compile_caps(oldops);auto oldcopy=oldp.words;auto newp=compile_caps(newops);uint16_t oldid=bt.intern(oldops),newid=bt.intern(newops);bool immutable=(oldp.words==oldcopy)&&(oldid!=newid)&&decode_caps(bt.get(oldid))==oldops&&decode_caps(bt.get(newid))==newops;
    std::ofstream hr(out/"historical_recovery_audit.csv");hr<<"old_block,new_block,old_words,new_words,old_immutable,both_uniform\n"<<oldid<<','<<newid<<','<<oldp.words.size()<<','<<newp.words.size()<<','<<immutable<<','<<(std::all_of(decode_roots(oldp).begin(),decode_roots(oldp).end(),is_root)&&std::all_of(decode_roots(newp).begin(),decode_roots(newp).end(),is_root))<<"\n";

    // Emit sample assembly-like dump.
    std::ofstream ad(out/"sample_odc_assembly.txt");ad<<"# 16-bit DRM-ISA; only semantic opcodes OBS/DRV/CMT\n# task: daily_http\n"<<dump(compile_caps(seq({"http.request","transform.extract","transform.summarize","fs.write","notify.send"})));

    double byte_red=graph_total?1.0-double(byte_total)/graph_total:0;double fused_red=graph_total?1.0-double(fused_total)/graph_total:0;
    std::ofstream js(out/"summary.json");js<<std::fixed<<std::setprecision(6)<<"{\n"
      <<"  \"episodes_learned\": "<<all.size()<<",\n"
      <<"  \"final_tasks\": "<<final_tasks.size()<<",\n"
      <<"  \"derived_words\": "<<planner.vocab.derived.size()<<",\n"
      <<"  \"semantic_total\": "<<semantic_total<<",\n"
      <<"  \"recoveries\": "<<recoveries<<",\n"
      <<"  \"local_repairs\": "<<repairs<<",\n"
      <<"  \"structural_changes\": "<<structural_changes<<",\n"
      <<"  \"roundtrip_tasks\": "<<roundtrip_tasks<<",\n"
      <<"  \"root_equivalent_tasks\": "<<root_equivalent_tasks<<",\n"
      <<"  \"dense_roundtrip_tasks\": "<<dense_roundtrip_tasks<<",\n"
      <<"  \"semantic_opcodes\": 3,\n"
      <<"  \"explicit_odc_instruction_bits\": 16,\n"
      <<"  \"dense_microop_instruction_bits\": 8,\n"
      <<"  \"graph_bytes\": "<<graph_total<<",\n"
      <<"  \"bytecode_bytes\": "<<byte_total<<",\n"
      <<"  \"bytecode_reduction_vs_graph\": "<<byte_red<<",\n"
      <<"  \"dense_microop_bytes\": "<<dense_total<<",\n"
      <<"  \"dense_microop_reduction_vs_graph\": "<<(graph_total?1.0-double(dense_total)/graph_total:0)<<",\n"
      <<"  \"unique_blocks\": "<<bt.blocks.size()<<",\n"
      <<"  \"block_table_bytes\": "<<block_table_bytes<<",\n"
      <<"  \"task_reference_bytes\": "<<task_refs<<",\n"
      <<"  \"fused_total_bytes\": "<<fused_total<<",\n"
      <<"  \"fused_reduction_vs_graph\": "<<fused_red<<",\n"
      <<"  \"dense_block_table_bytes\": "<<dense_block_table_bytes<<",\n"
      <<"  \"dense_fused_total_bytes\": "<<dense_fused_total<<",\n"
      <<"  \"dense_fused_reduction_vs_graph\": "<<(graph_total?1.0-double(dense_fused_total)/graph_total:0)<<",\n"
      <<"  \"graph_dispatch_ns\": "<<ds.graph_ns<<",\n"
      <<"  \"bytecode_dispatch_ns\": "<<ds.bytecode_ns<<",\n"
      <<"  \"block_dispatch_ns\": "<<ds.block_ns<<",\n"
      <<"  \"bytecode_dispatch_speedup\": "<<(ds.graph_ns/ds.bytecode_ns)<<",\n"
      <<"  \"block_dispatch_speedup\": "<<(ds.graph_ns/ds.block_ns)<<",\n"
      <<"  \"dense_dispatch_ns\": "<<ds.dense_ns<<",\n"
      <<"  \"dense_block_dispatch_ns\": "<<ds.dense_block_ns<<",\n"
      <<"  \"dense_dispatch_speedup\": "<<(ds.graph_ns/ds.dense_ns)<<",\n"
      <<"  \"dense_block_dispatch_speedup\": "<<(ds.graph_ns/ds.dense_block_ns)<<",\n"
      <<"  \"live_workflows\": "<<subset.size()<<",\n"
      <<"  \"live_success_all_modes\": "<<equiv<<",\n"
      <<"  \"graph_live_ms\": "<<gms<<",\n"
      <<"  \"bytecode_live_ms\": "<<bms<<",\n"
      <<"  \"block_live_ms\": "<<fms<<",\n"
      <<"  \"dense_live_ms\": "<<dms<<",\n"
      <<"  \"dense_block_live_ms\": "<<dfms<<",\n"
      <<"  \"historical_blocks_immutable\": "<<(immutable?"true":"false")<<",\n"
      <<"  \"uniform_vocabulary\": "<<(all_uniform?"true":"false")<<",\n"
      <<"  \"root_vocabulary\": [\"OBSERVE\", \"DERIVE\", \"COMMIT\"]\n}\n";

    std::cout<<"tasks="<<final_tasks.size()<<" graph_bytes="<<graph_total<<" bytecode_bytes="<<byte_total<<" fused_bytes="<<fused_total
             <<" dispatch_speedup="<<(ds.graph_ns/ds.bytecode_ns)<<" block_speedup="<<(ds.graph_ns/ds.block_ns)<<" dense_speedup="<<(ds.graph_ns/ds.dense_ns)<<" dense_block_speedup="<<(ds.graph_ns/ds.dense_block_ns)
             <<" live="<<equiv<<"/"<<subset.size()<<" derived="<<planner.vocab.derived.size()<<" uniform="<<all_uniform<<" immutable_history="<<immutable<<"\n";
    return (all_uniform&&immutable&&equiv==subset.size()&&gok==subset.size()&&bok==subset.size()&&fok==subset.size()&&dok==subset.size()&&dfok==subset.size())?0:2;
}

} // namespace bc

int main(int argc,char**argv){try{if(argc>1&&std::string(argv[1])=="--self-test"){bool ok=bc::self_test();std::cout<<(ok?"BYTECODE_SELF_TEST_PASS":"BYTECODE_SELF_TEST_FAIL")<<"\n";return ok?0:1;}fs::path out="results";for(int i=1;i+1<argc;++i)if(std::string(argv[i])=="--out")out=argv[i+1];return bc::run(out);}catch(const std::exception&e){std::cerr<<"fatal: "<<e.what()<<"\n";return 1;}}
