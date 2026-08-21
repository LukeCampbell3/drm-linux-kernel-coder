#include <algorithm>
#include <array>
#include <atomic>
#include <chrono>
#include <cmath>
#include <csignal>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <fcntl.h>
#include <filesystem>
#include <fstream>
#include <functional>
#include <iomanip>
#include <iostream>
#include <map>
#include <netinet/in.h>
#include <poll.h>
#include <set>
#include <sstream>
#include <string>
#include <sys/resource.h>
#include <sys/socket.h>
#include <sys/stat.h>
#include <sys/types.h>
#include <sys/un.h>
#include <sys/wait.h>
#include <thread>
#include <unordered_map>
#include <utility>
#include <vector>
#include <arpa/inet.h>
#include <unistd.h>

namespace fs = std::filesystem;
using Seq = std::vector<std::string>;
using Clock = std::chrono::steady_clock;
static constexpr std::array<const char*,3> ROOT = {"OBSERVE","DERIVE","COMMIT"};

static const std::map<std::string, Seq> CAP_ROOT = {
    {"fs.read", {"OBSERVE"}},
    {"state.read", {"OBSERVE"}},
    {"proc.observe", {"OBSERVE"}},
    {"timer.observe", {"OBSERVE"}},
    {"http.request", {"DERIVE","COMMIT","OBSERVE"}},
    {"ipc.request", {"DERIVE","COMMIT","OBSERVE"}},
    {"process.run", {"DERIVE","COMMIT","OBSERVE"}},
    {"transform.extract", {"DERIVE"}},
    {"transform.summarize", {"DERIVE"}},
    {"fs.write", {"DERIVE","COMMIT"}},
    {"state.write", {"DERIVE","COMMIT"}},
    {"notify.send", {"DERIVE","COMMIT"}},
};

static bool is_root(const std::string& s) {
    return std::find_if(ROOT.begin(), ROOT.end(), [&](const char* x){return s==x;}) != ROOT.end();
}
static bool known_cap(const std::string& s){ return CAP_ROOT.contains(s); }
static Seq seq(std::initializer_list<const char*> xs){ Seq v; for(auto* x:xs)v.emplace_back(x); return v; }

struct Episode {
    size_t idx{};
    std::string task, phase;
    Seq ops;
    std::string source, output, url_path;
    bool ancestral{false};
};

struct PlanMetrics {
    size_t semantic{}, recovery{}, local_repair{}, structural_change{}, derived{}, active{}, structure_bytes{};
    double avg_depth{};
    size_t max_depth{};
    bool uniform{true};
};

struct Vocabulary {
    std::map<std::string, Seq> derived;
    size_t counter{};

    Seq expand_symbol_inner(const std::string& sym, std::set<std::string>& stack) const {
        if(known_cap(sym)) return {sym};
        if(stack.contains(sym)) throw std::runtime_error("cycle:"+sym);
        auto it=derived.find(sym); if(it==derived.end()) throw std::runtime_error("unknown:"+sym);
        stack.insert(sym); Seq out;
        for(const auto& part:it->second){ auto z=expand_symbol_inner(part,stack); out.insert(out.end(),z.begin(),z.end()); }
        stack.erase(sym); return out;
    }
    Seq expand_symbol(const std::string& sym) const { std::set<std::string> st; return expand_symbol_inner(sym,st); }
    Seq expand_root(const std::string& sym) const {
        Seq out; for(const auto& c:expand_symbol(sym)){ auto it=CAP_ROOT.find(c); if(it==CAP_ROOT.end()) throw std::runtime_error("unknown cap"); out.insert(out.end(),it->second.begin(),it->second.end()); } return out;
    }
    size_t depth_inner(const std::string& sym, std::set<std::string>& stack) const {
        if(known_cap(sym)) return 0;
        if(stack.contains(sym)) throw std::runtime_error("cycle:"+sym);
        auto it=derived.find(sym); if(it==derived.end()) throw std::runtime_error("unknown:"+sym);
        stack.insert(sym); size_t m=0; for(const auto& p:it->second)m=std::max(m,depth_inner(p,stack)); stack.erase(sym); return 1+m;
    }
    size_t depth(const std::string& sym) const { std::set<std::string> st; return depth_inner(sym,st); }
    bool audit() const {
        try { for(const auto& [name,_]:derived){ auto roots=expand_root(name); if(roots.empty()) return false; for(const auto& r:roots) if(!is_root(r)) return false; } }
        catch(...) { return false; } return true;
    }
    std::vector<std::pair<std::string,Seq>> expansions() const {
        std::vector<std::pair<std::string,Seq>> out; for(const auto& [k,_]:derived)out.push_back({k,expand_symbol(k)});
        std::sort(out.begin(),out.end(),[](const auto&a,const auto&b){ if(a.second.size()!=b.second.size())return a.second.size()>b.second.size(); return a.first<b.first; }); return out;
    }
    Seq compress_with(const Seq& in, std::vector<std::pair<std::string,Seq>> extra={}) const {
        auto ex=expansions(); extra.insert(extra.end(),ex.begin(),ex.end());
        std::sort(extra.begin(),extra.end(),[](const auto&a,const auto&b){ if(a.second.size()!=b.second.size())return a.second.size()>b.second.size(); return a.first<b.first; });
        Seq out; size_t i=0; while(i<in.size()){
            bool hit=false; for(const auto& [name,p]:extra){ if(p.size()<2 || i+p.size()>in.size())continue; if(std::equal(p.begin(),p.end(),in.begin()+static_cast<long>(i))){out.push_back(name);i+=p.size();hit=true;break;} }
            if(!hit){out.push_back(in[i]);++i;}
        } return out;
    }
    Seq compress(const Seq& in) const {return compress_with(in);}
};

struct DrmPlanner {
    Vocabulary vocab;
    size_t active_cap{8};
    long mdl_threshold{3};
    std::map<std::string,Seq> active, history;
    std::vector<std::string> lru;
    std::map<std::string,size_t> history_version;
    std::map<Seq,std::set<std::string>> subseq_users;
    size_t version{};

    void touch(const std::string& task,const Seq& s){ active[task]=s; lru.erase(std::remove(lru.begin(),lru.end(),task),lru.end()); lru.push_back(task); while(lru.size()>active_cap){active.erase(lru.front());lru.erase(lru.begin());}}
    std::set<Seq> note_subseqs(const std::string& task,const Seq& s){ std::set<Seq> touched; size_t mx=std::min<size_t>(5,s.size()); for(size_t n=2;n<=mx;n++)for(size_t i=0;i+n<=s.size();i++){ Seq cand(s.begin()+static_cast<long>(i),s.begin()+static_cast<long>(i+n)); subseq_users[cand].insert(task); touched.insert(std::move(cand)); } return touched; }
    size_t corpus_cost(std::vector<std::pair<std::string,Seq>> extra={}) const { size_t n=0; for(const auto& [_,s]:history)n+=vocab.compress_with(s,extra).size(); return n; }
    size_t maybe_grow(const std::set<Seq>& candidates){
        if(candidates.empty()) return 0;
        size_t baseline=corpus_cost(); std::set<Seq> existing; for(const auto& [k,_]:vocab.derived)existing.insert(vocab.expand_symbol(k));
        bool found=false; std::tuple<long,size_t,size_t> best_key{}; Seq best_def;
        for(const auto& cand:candidates){ auto uit=subseq_users.find(cand); if(uit==subseq_users.end())continue; const auto& userset=uit->second; if(userset.size()<2 || existing.contains(cand))continue; Seq def=vocab.compress(cand); if(def.size()<=1)continue; size_t new_cost=corpus_cost({{"__new__",cand}}); long gain=static_cast<long>(baseline)-static_cast<long>(new_cost)-static_cast<long>(def.size()+1); if(gain<mdl_threshold)continue; auto key=std::make_tuple(gain,userset.size(),cand.size()); if(!found||key>best_key){found=true;best_key=key;best_def=def;} }
        if(found){ ++vocab.counter; char buf[16]; std::snprintf(buf,sizeof(buf),"d%03zu",vocab.counter); vocab.derived[buf]=best_def; return 1;} return 0;
    }
    static Seq diff_middle(const Seq& old,const Seq& now){ size_t p=0;while(p<std::min(old.size(),now.size())&&old[p]==now[p])p++;size_t s=0;while(s<std::min(old.size()-p,now.size()-p)&&old[old.size()-1-s]==now[now.size()-1-s])s++; size_t end=s?now.size()-s:now.size();return Seq(now.begin()+static_cast<long>(p),now.begin()+static_cast<long>(end)); }
    size_t structure_bytes() const { size_t n=0; for(auto*x:ROOT)n+=std::strlen(x); for(const auto&[k,v]:vocab.derived){n+=k.size()+1;for(const auto&s:v)n+=s.size()+1;} for(const auto&[k,v]:active){n+=k.size()+1;for(const auto&s:v)n+=s.size()+1;} for(const auto&[k,v]:history_version)n+=k.size()+std::to_string(v).size()+2; return n; }
    PlanMetrics plan(const Episode& ep){
        PlanMetrics m; auto ai=active.find(ep.task); auto hi=history.find(ep.task);
        if(ai!=active.end()){
            if(ai->second==ep.ops)m.semantic=1; else {auto d=diff_middle(ai->second,ep.ops);m.semantic=std::max<size_t>(1,vocab.compress(d).size());m.local_repair=1;m.structural_change++;}
        } else if(hi!=history.end()){
            if(ep.ancestral){m.recovery=1;m.semantic=std::max<size_t>(1,vocab.compress(ep.ops).size());m.structural_change++;}
            else if(hi->second!=ep.ops){auto d=diff_middle(hi->second,ep.ops);m.semantic=std::max<size_t>(1,vocab.compress(d).size());m.local_repair=1;m.structural_change++;}
            else m.semantic=1;
        } else {m.semantic=std::max<size_t>(1,vocab.compress(ep.ops).size());m.structural_change++;}
        const bool new_structural_evidence = (hi==history.end()) || (hi->second!=ep.ops);
        version++;history[ep.task]=ep.ops;history_version[ep.task]=version;
        if(new_structural_evidence){ auto touched=note_subseqs(ep.task,ep.ops); m.structural_change+=maybe_grow(touched); }
        touch(ep.task,ep.ops);
        m.derived=vocab.derived.size();m.active=active.size();m.uniform=vocab.audit();m.structure_bytes=structure_bytes();
        if(!vocab.derived.empty()){size_t sum=0;for(const auto&[k,_]:vocab.derived){auto d=vocab.depth(k);sum+=d;m.max_depth=std::max(m.max_depth,d);}m.avg_depth=static_cast<double>(sum)/vocab.derived.size();}
        return m;
    }
};

struct Baseline {
    std::string kind; std::map<std::string,Seq> seen; size_t structure_bytes()const{size_t n=0;for(const auto&[k,v]:seen){n+=k.size()+1;for(const auto&s:v)n+=s.size()+1;}return n;}
    PlanMetrics plan(const Episode& ep){ PlanMetrics m; if(kind=="stateless")m.semantic=ep.ops.size(); else {auto it=seen.find(ep.task);if(it==seen.end()){m.semantic=ep.ops.size();m.structural_change=1;}else if(it->second==ep.ops)m.semantic=1;else if(kind=="checkpoint_replay"){auto d=DrmPlanner::diff_middle(it->second,ep.ops);m.semantic=std::max<size_t>(1,d.size());m.local_repair=1;}else{m.semantic=ep.ops.size();m.structural_change=1;} if(ep.ancestral&&kind=="checkpoint_replay")m.recovery=1;seen[ep.task]=ep.ops;m.structure_bytes=structure_bytes();} return m; }
};

struct TcpServer {
    int fd{-1}; uint16_t port{}; std::atomic<bool> stop{false}; std::thread th;
    TcpServer(){fd=::socket(AF_INET,SOCK_STREAM,0);if(fd<0)throw std::runtime_error("socket");int one=1;setsockopt(fd,SOL_SOCKET,SO_REUSEADDR,&one,sizeof(one));sockaddr_in a{};a.sin_family=AF_INET;a.sin_addr.s_addr=htonl(INADDR_LOOPBACK);a.sin_port=0;if(bind(fd,reinterpret_cast<sockaddr*>(&a),sizeof(a))<0)throw std::runtime_error("bind");socklen_t n=sizeof(a);getsockname(fd,reinterpret_cast<sockaddr*>(&a),&n);port=ntohs(a.sin_port);listen(fd,8);th=std::thread([this]{loop();});}
    void loop(){while(!stop){pollfd p{fd,POLLIN,0};int r=poll(&p,1,20);if(r<=0)continue;int c=accept(fd,nullptr,nullptr);if(c<0)continue;char b[1024];ssize_t n=read(c,b,sizeof(b));int idx=0;if(n>0){std::string q(b,b+n);auto pos=q.find("/news_");if(pos!=std::string::npos)idx=std::atoi(q.c_str()+static_cast<long>(pos+6));}std::ostringstream body;body<<"<html><body><h1>News "<<idx<<"</h1><p>";for(int j=1;j<36;j++)body<<"Story"<<idx<<"-"<<j<<" DRM local scheduling repeated task optimization Linux ";body<<"</p></body></html>";auto s=body.str();std::ostringstream resp;resp<<"HTTP/1.1 200 OK\r\nContent-Length: "<<s.size()<<"\r\nConnection: close\r\n\r\n"<<s;auto x=resp.str();write(c,x.data(),x.size());close(c);} }
    ~TcpServer(){stop=true;if(th.joinable())th.join();if(fd>=0)close(fd);}
};

struct UnixServer {
    int fd{-1}; fs::path path; std::atomic<bool> stop{false}; std::thread th;
    explicit UnixServer(fs::path p):path(std::move(p)){::unlink(path.c_str());fd=::socket(AF_UNIX,SOCK_STREAM,0);if(fd<0)throw std::runtime_error("unix socket");sockaddr_un a{};a.sun_family=AF_UNIX;std::snprintf(a.sun_path,sizeof(a.sun_path),"%s",path.c_str());if(bind(fd,reinterpret_cast<sockaddr*>(&a),sizeof(a))<0)throw std::runtime_error("unix bind");listen(fd,8);th=std::thread([this]{loop();});}
    void loop(){while(!stop){pollfd p{fd,POLLIN,0};int r=poll(&p,1,20);if(r<=0)continue;int c=accept(fd,nullptr,nullptr);if(c<0)continue;char b[512];ssize_t n=read(c,b,sizeof(b));std::string in=n>0?std::string(b,b+n):"";std::string out="ipc-ok:"+in;write(c,out.data(),out.size());close(c);} }
    ~UnixServer(){stop=true;if(th.joinable())th.join();if(fd>=0)close(fd);::unlink(path.c_str());}
};

struct LiveExecutor {
    fs::path work; uint16_t port; fs::path unix_path; size_t state_runs{},commits{},process_spawns{},tcp_requests{},ipc_requests{},timer_events{};std::map<std::string,size_t> root_counts;
    LiveExecutor(fs::path w,uint16_t p,fs::path up):work(std::move(w)),port(p),unix_path(std::move(up)){}
    void roots(const std::string& cap){for(const auto&r:CAP_ROOT.at(cap))root_counts[r]++;}
    std::string http_get(const std::string& path){int s=socket(AF_INET,SOCK_STREAM,0);if(s<0)throw std::runtime_error("socket");sockaddr_in a{};a.sin_family=AF_INET;a.sin_port=htons(port);inet_pton(AF_INET,"127.0.0.1",&a.sin_addr);if(connect(s,reinterpret_cast<sockaddr*>(&a),sizeof(a))<0){close(s);throw std::runtime_error("connect");}std::string q="GET "+path+" HTTP/1.1\r\nHost:127.0.0.1\r\nConnection:close\r\n\r\n";write(s,q.data(),q.size());std::string r;char b[2048];for(;;){ssize_t n=read(s,b,sizeof(b));if(n<=0)break;r.append(b,b+n);}close(s);tcp_requests++;auto pos=r.find("\r\n\r\n");return pos==std::string::npos?r:r.substr(pos+4);}
    std::string unix_roundtrip(const std::string& in){int s=socket(AF_UNIX,SOCK_STREAM,0);if(s<0)throw std::runtime_error("unix socket");sockaddr_un a{};a.sun_family=AF_UNIX;std::snprintf(a.sun_path,sizeof(a.sun_path),"%s",unix_path.c_str());if(connect(s,reinterpret_cast<sockaddr*>(&a),sizeof(a))<0){close(s);throw std::runtime_error("unix connect");}write(s,in.data(),in.size());char b[1024];ssize_t n=read(s,b,sizeof(b));close(s);ipc_requests++;return n>0?std::string(b,b+n):"";}
    std::string run_hash(const fs::path& p){int fds[2];if(pipe(fds)<0)throw std::runtime_error("pipe");pid_t pid=fork();if(pid<0)throw std::runtime_error("fork");if(pid==0){dup2(fds[1],STDOUT_FILENO);close(fds[0]);close(fds[1]);execlp("sha256sum","sha256sum",p.c_str(),static_cast<char*>(nullptr));_exit(127);}close(fds[1]);std::string out;char b[512];for(;;){ssize_t n=read(fds[0],b,sizeof(b));if(n<=0)break;out.append(b,b+n);}close(fds[0]);int st=0;waitpid(pid,&st,0);process_spawns++;if(!WIFEXITED(st)||WEXITSTATUS(st)!=0)throw std::runtime_error("child failed");return out;}
    static std::string extract(std::string s){std::string o;bool tag=false;for(char c:s){if(c=='<'){tag=true;continue;}if(c=='>'){tag=false;o.push_back(' ');continue;}if(!tag)o.push_back(c);}std::istringstream in(o);std::ostringstream r;std::string w;bool first=true;while(in>>w){if(!first)r<<' ';r<<w;first=false;}return r.str();}
    static std::string summarize(const std::string& s){std::istringstream in(s);std::vector<std::string>w;std::string x;while(in>>x)w.push_back(x);std::ostringstream o;o<<"words="<<w.size()<<" head=";for(size_t i=0;i<std::min<size_t>(10,w.size());i++){if(i)o<<' ';o<<w[i];}return o.str();}
    bool execute(const Episode& ep,std::string& err){std::string data;try{for(const auto& cap:ep.ops){roots(cap);if(cap=="fs.read")data=(std::ostringstream{}<<std::ifstream(work/ep.source).rdbuf()).str();else if(cap=="state.read"){std::ifstream f(work/"state.txt");data=f? (std::ostringstream{}<<f.rdbuf()).str() : "runs=0";}else if(cap=="proc.observe"){std::ifstream f("/proc/self/status");data=(std::ostringstream{}<<f.rdbuf()).str();}else if(cap=="timer.observe"){auto until=Clock::now()+std::chrono::milliseconds(2);while(Clock::now()<until)std::this_thread::sleep_for(std::chrono::microseconds(200));data="timer-fired";timer_events++;}else if(cap=="http.request")data=http_get(ep.url_path);else if(cap=="ipc.request")data=unix_roundtrip(data.empty()?ep.task:data.substr(0,std::min<size_t>(80,data.size())));else if(cap=="process.run")data=run_hash(work/ep.source);else if(cap=="transform.extract")data=extract(data);else if(cap=="transform.summarize")data=summarize(data);else if(cap=="fs.write"){auto out=work/ep.output;fs::create_directories(out.parent_path());auto tmp=out;tmp+=".candidate";{std::ofstream f(tmp);f<<data;}fs::rename(tmp,out);commits++;}else if(cap=="state.write"){state_runs++;auto tmp=work/"state.candidate";{std::ofstream f(tmp);f<<"runs="<<state_runs<<" last="<<data.substr(0,120);}fs::rename(tmp,work/"state.txt");commits++;}else if(cap=="notify.send"){std::ofstream f(work/"notifications.log",std::ios::app);f<<data.substr(0,300)<<"\n";commits++;}else throw std::runtime_error("unknown cap "+cap);}if(std::find(ep.ops.begin(),ep.ops.end(),"fs.write")!=ep.ops.end()){auto p=work/ep.output;if(!fs::exists(p)||fs::file_size(p)==0)throw std::runtime_error("verify output");}return true;}catch(const std::exception&e){err=e.what();return false;}}
};

static void make_fixtures(const fs::path& w){fs::remove_all(w);fs::create_directories(w/"inputs");fs::create_directories(w/"outputs");for(int i=0;i<16;i++){std::ofstream f(w/"inputs"/("report_"+std::to_string(i)+".csv"));f<<"kind,id,label,value\n";for(int j=1;j<60;j++)f<<"item,"<<j<<",value,"<<(i*j+3)<<"\n";}}
static void add(std::vector<Episode>&v,size_t&idx,std::string task,std::string phase,Seq ops,std::string src="inputs/report_0.csv",std::string out="",std::string url="/news_0.html",bool anc=false){idx++;if(out.empty())out="outputs/"+task+".txt";v.push_back({idx,std::move(task),std::move(phase),std::move(ops),std::move(src),std::move(out),std::move(url),anc});}
static std::vector<Episode> workload(){
    std::vector<Episode> v;size_t i=0;
    Seq file=seq({"fs.read","transform.summarize","fs.write","notify.send"});
    Seq hash=seq({"process.run","transform.summarize","fs.write","notify.send"});
    Seq http=seq({"http.request","transform.extract","transform.summarize","fs.write","notify.send"});
    Seq state=seq({"state.read","transform.summarize","state.write","notify.send"});
    Seq ipc=seq({"fs.read","transform.summarize","ipc.request","fs.write"});
    Seq proc=seq({"proc.observe","transform.extract","transform.summarize","fs.write"});
    Seq timer=seq({"timer.observe","state.read","transform.summarize","state.write"});
    for(int r=0;r<3;r++){add(v,i,"daily_file","warmup",file,"inputs/report_"+std::to_string(r)+".csv");add(v,i,"daily_hash","warmup",hash,"inputs/report_"+std::to_string(r+1)+".csv");add(v,i,"daily_http","warmup",http,"inputs/report_0.csv","","/news_"+std::to_string(r)+".html");add(v,i,"daily_state","warmup",state);add(v,i,"daily_ipc","warmup",ipc,"inputs/report_"+std::to_string(r+2)+".csv");add(v,i,"daily_proc","warmup",proc);add(v,i,"daily_timer","warmup",timer);}
    std::vector<Seq> combos={
      seq({"fs.read","transform.extract","transform.summarize","fs.write","notify.send"}),
      seq({"http.request","transform.extract","transform.summarize","state.write","notify.send"}),
      seq({"process.run","transform.extract","transform.summarize","fs.write"}),
      seq({"state.read","transform.extract","transform.summarize","fs.write","notify.send"}),
      seq({"fs.read","transform.summarize","ipc.request","state.write","notify.send"}),
      seq({"proc.observe","transform.extract","transform.summarize","ipc.request","fs.write"}),
      seq({"timer.observe","state.read","transform.summarize","fs.write","notify.send"}),
      seq({"http.request","transform.extract","ipc.request","transform.summarize","fs.write"})
    };
    for(int n=0;n<40;n++)add(v,i,std::string("novel_")+(n<10?"0":"")+std::to_string(n),"novel",combos[n%combos.size()],"inputs/report_"+std::to_string(n%16)+".csv","","/news_"+std::to_string(n%8)+".html");
    auto snap=v;for(int n=0;n<16;n++){std::string t=std::string("novel_")+(n<10?"0":"")+std::to_string(n);auto it=std::find_if(snap.begin(),snap.end(),[&](const Episode&e){return e.task==t;});add(v,i,it->task,"repeat",it->ops,it->source,it->output,it->url_path);}
    add(v,i,"daily_file","drift",seq({"fs.read","transform.extract","transform.summarize","fs.write","notify.send"}),"inputs/report_13.csv");
    add(v,i,"daily_http","drift",seq({"http.request","transform.extract","transform.summarize","state.write","notify.send"}),"inputs/report_0.csv","","/news_7.html");
    add(v,i,"daily_hash","drift",seq({"process.run","transform.summarize","ipc.request","state.write","notify.send"}),"inputs/report_14.csv");
    add(v,i,"daily_ipc","drift",seq({"fs.read","transform.extract","transform.summarize","ipc.request","fs.write"}),"inputs/report_15.csv");
    for(int n=0;n<10;n++)add(v,i,"tail_"+std::to_string(n),"evict",combos[n%combos.size()],"inputs/report_"+std::to_string((n+3)%16)+".csv","","/news_"+std::to_string(n%8)+".html");
    for(const std::string t:{"daily_http","daily_file","daily_hash","daily_ipc"}){auto it=std::find_if(snap.begin(),snap.end(),[&](const Episode&e){return e.task==t;});add(v,i,it->task,"ancestral",it->ops,it->source,it->output,it->url_path,true);add(v,i,it->task,"post_recovery",it->ops,it->source,it->output,it->url_path,false);}
    return v;
}

struct RUsage {double u{},s{};long maxrss{};};
static RUsage usage(){rusage a{},b{};getrusage(RUSAGE_SELF,&a);getrusage(RUSAGE_CHILDREN,&b);auto sec=[](timeval t){return t.tv_sec+t.tv_usec/1e6;};return{sec(a.ru_utime)+sec(b.ru_utime),sec(a.ru_stime)+sec(b.ru_stime),std::max(a.ru_maxrss,b.ru_maxrss)};}
struct IO {long long read_bytes{},write_bytes{};};
static IO io(){std::ifstream f("/proc/self/io");std::string k;long long v;IO x;while(f>>k>>v){if(k=="read_bytes:")x.read_bytes=v;if(k=="write_bytes:")x.write_bytes=v;}return x;}
static long rss_kb(){std::ifstream f("/proc/self/status");std::string k;long v;while(f>>k){if(k=="VmRSS:"){f>>v;return v;}std::string rest;std::getline(f,rest);}return 0;}
static std::string esc(const std::string&s){if(s.find_first_of(",\"\n")==std::string::npos)return s;std::string o="\"";for(char c:s){if(c=='\"')o+="\"\"";else o+=c;}return o+"\"";}

static bool self_test(){
    if(std::string(ROOT[0])!="OBSERVE"||std::string(ROOT[1])!="DERIVE"||std::string(ROOT[2])!="COMMIT")return false;
    for(const auto&[_,r]:CAP_ROOT)for(const auto&x:r)if(!is_root(x))return false;
    Vocabulary v;v.derived["d001"]=seq({"transform.summarize","fs.write"});v.derived["d002"]={"fs.read","d001"};if(!v.audit())return false;auto roots=v.expand_root("d002");if(roots!=seq({"OBSERVE","DERIVE","DERIVE","COMMIT"}))return false;
    DrmPlanner p;Episode e{1,"old","x",seq({"fs.read","transform.summarize","fs.write"}),"x","y","/",false};p.active_cap=1;p.plan(e);auto o=e;o.task="other";p.plan(o);auto r=e;r.ancestral=true;if(p.plan(r).recovery!=1)return false;if(p.plan(e).recovery!=0)return false;return true;
}

static int run(const fs::path& out){
    fs::create_directories(out);auto work=out/"workspace";make_fixtures(work);TcpServer tcp;UnixServer uds(work/"drm.sock");LiveExecutor ex(work,tcp.port,work/"drm.sock");DrmPlanner drm;auto eps=workload();
    std::ofstream tr(out/"live_trace.csv");tr<<"episode,task,phase,success,wall_ms,planner_ms,executor_ms,cpu_user_ms,cpu_sys_ms,rss_kb,read_bytes,write_bytes,semantic,recovery,local_repair,structural_change,derived,active,structure_bytes,avg_depth,max_depth,uniform\n";
    size_t success=0,sem=0,rec=0,rep=0,chg=0;double total_wall=0;long peak=0;auto global=Clock::now();
    for(const auto&e:eps){auto u0=usage();auto i0=io();auto t0=Clock::now();auto tp0=Clock::now();auto pm=drm.plan(e);auto tp1=Clock::now();std::string err;auto te0=Clock::now();bool ok=ex.execute(e,err);auto te1=Clock::now();auto t1=Clock::now();auto u1=usage();auto i1=io();double wall=std::chrono::duration<double,std::milli>(t1-t0).count();double planner_ms=std::chrono::duration<double,std::milli>(tp1-tp0).count();double executor_ms=std::chrono::duration<double,std::milli>(te1-te0).count();long rss=rss_kb();peak=std::max(peak,rss);success+=ok;sem+=pm.semantic;rec+=pm.recovery;rep+=pm.local_repair;chg+=pm.structural_change;total_wall+=wall;
      tr<<e.idx<<','<<esc(e.task)<<','<<e.phase<<','<<ok<<','<<std::fixed<<std::setprecision(3)<<wall<<','<<planner_ms<<','<<executor_ms<<','<<(u1.u-u0.u)*1000<<','<<(u1.s-u0.s)*1000<<','<<rss<<','<<(i1.read_bytes-i0.read_bytes)<<','<<(i1.write_bytes-i0.write_bytes)<<','<<pm.semantic<<','<<pm.recovery<<','<<pm.local_repair<<','<<pm.structural_change<<','<<pm.derived<<','<<pm.active<<','<<pm.structure_bytes<<','<<pm.avg_depth<<','<<pm.max_depth<<','<<pm.uniform<<"\n"; }
    std::ofstream va(out/"vocabulary_audit.csv");va<<"name,definition,capability_expansion,root_expansion,depth,uniform\n";size_t raw_tokens=0,compressed_tokens=0,def_tokens=0;for(const auto&[_,s]:drm.history){raw_tokens+=s.size();compressed_tokens+=drm.vocab.compress(s).size();}for(const auto&[name,d]:drm.vocab.derived){def_tokens+=d.size();auto c=drm.vocab.expand_symbol(name);auto r=drm.vocab.expand_root(name);auto join=[](const Seq&s){std::string o;for(size_t i=0;i<s.size();i++){if(i)o+=" > ";o+=s[i];}return o;};bool uni=std::all_of(r.begin(),r.end(),is_root);va<<name<<','<<esc(join(d))<<','<<esc(join(c))<<','<<esc(join(r))<<','<<drm.vocab.depth(name)<<','<<uni<<"\n";}
    std::ofstream bc(out/"baseline_comparison.csv");bc<<"system,episodes,semantic_total,semantic_mean,recoveries,local_repairs,structural_changes,final_structure_bytes\n";for(const std::string k:{"stateless","template_cache","checkpoint_replay"}){Baseline b; b.kind=k;size_t s=0,r=0,p=0,c=0;PlanMetrics last;for(const auto&e:eps){last=b.plan(e);s+=last.semantic;r+=last.recovery;p+=last.local_repair;c+=last.structural_change;}bc<<k<<','<<eps.size()<<','<<s<<','<<std::setprecision(6)<<double(s)/eps.size()<<','<<r<<','<<p<<','<<c<<','<<last.structure_bytes<<"\n";}bc<<"drm_odc_cpp,"<<eps.size()<<','<<sem<<','<<double(sem)/eps.size()<<','<<rec<<','<<rep<<','<<chg<<','<<drm.structure_bytes()<<"\n";
    long vocab_headers=static_cast<long>(drm.vocab.derived.size());long compressed_total=static_cast<long>(compressed_tokens+def_tokens+vocab_headers);double dl_red=raw_tokens?1.0-double(compressed_total)/raw_tokens:0;double global_ms=std::chrono::duration<double,std::milli>(Clock::now()-global).count();
    std::ofstream js(out/"summary.json");js<<std::fixed<<std::setprecision(6)<<"{\n"<<"  \"compiler\": \"g++ "<<__GNUC__<<'.'<<__GNUC_MINOR__<<"\",\n"<<"  \"episodes\": "<<eps.size()<<",\n"<<"  \"success_rate\": "<<double(success)/eps.size()<<",\n"<<"  \"semantic_total\": "<<sem<<",\n"<<"  \"semantic_mean\": "<<double(sem)/eps.size()<<",\n"<<"  \"derived_final\": "<<drm.vocab.derived.size()<<",\n"<<"  \"structure_bytes_final\": "<<drm.structure_bytes()<<",\n"<<"  \"uniform_vocabulary\": "<<(drm.vocab.audit()?"true":"false")<<",\n"<<"  \"recoveries\": "<<rec<<",\n"<<"  \"local_repairs\": "<<rep<<",\n"<<"  \"commits\": "<<ex.commits<<",\n"<<"  \"process_spawns\": "<<ex.process_spawns<<",\n"<<"  \"tcp_requests\": "<<ex.tcp_requests<<",\n"<<"  \"ipc_requests\": "<<ex.ipc_requests<<",\n"<<"  \"timer_events\": "<<ex.timer_events<<",\n"<<"  \"peak_rss_kb\": "<<peak<<",\n"<<"  \"episode_wall_ms_sum\": "<<total_wall<<",\n"<<"  \"benchmark_wall_ms\": "<<global_ms<<",\n"<<"  \"raw_task_tokens\": "<<raw_tokens<<",\n"<<"  \"compressed_task_tokens\": "<<compressed_tokens<<",\n"<<"  \"definition_tokens\": "<<def_tokens<<",\n"<<"  \"description_length_reduction\": "<<dl_red<<",\n"<<"  \"root_counts\": {\"OBSERVE\": "<<ex.root_counts["OBSERVE"]<<", \"DERIVE\": "<<ex.root_counts["DERIVE"]<<", \"COMMIT\": "<<ex.root_counts["COMMIT"]<<"},\n"<<"  \"root_vocabulary\": [\"OBSERVE\", \"DERIVE\", \"COMMIT\"]\n}\n";
    std::cout<<"episodes="<<eps.size()<<" success="<<success<<" semantic="<<sem<<" derived="<<drm.vocab.derived.size()<<" recoveries="<<rec<<" repairs="<<rep<<" struct="<<drm.structure_bytes()<<" rss_kb="<<peak<<" dl_reduction="<<dl_red<<"\n";return success==eps.size()&&drm.vocab.audit()?0:2;
}

int main(int argc,char**argv){try{if(argc>1&&std::string(argv[1])=="--self-test"){bool ok=self_test();std::cout<<(ok?"SELF_TEST_PASS":"SELF_TEST_FAIL")<<"\n";return ok?0:1;}fs::path out="results";for(int i=1;i+1<argc;i++)if(std::string(argv[i])=="--out")out=argv[i+1];return run(out);}catch(const std::exception&e){std::cerr<<"fatal: "<<e.what()<<"\n";return 1;}}
