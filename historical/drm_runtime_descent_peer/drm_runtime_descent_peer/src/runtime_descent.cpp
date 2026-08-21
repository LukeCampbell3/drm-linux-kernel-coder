#define main drm_base_embedded_main
#include "main.cpp"
#undef main

#include <numeric>
#include <optional>
#include <memory>
#include <spawn.h>
extern char **environ;

namespace runtime_descent {

static const std::vector<size_t> CHUNKS = {128,256,512,1024,4096,16384,65536};
static const std::vector<int> TIMER_US = {800,400,200,100,50,25};

enum class Dim { ReadChunk, WriteChunk, SocketChunk, SummaryImpl, ExtractImpl, ProcessImpl, TimerQuantum };
static const char* dim_name(Dim d){
    switch(d){
        case Dim::ReadChunk:return "read_chunk"; case Dim::WriteChunk:return "write_chunk";
        case Dim::SocketChunk:return "socket_chunk"; case Dim::SummaryImpl:return "summary_impl";
        case Dim::ExtractImpl:return "extract_impl"; case Dim::ProcessImpl:return "process_impl";
        case Dim::TimerQuantum:return "timer_quantum";
    }
    return "unknown";
}

struct Theta {
    int read_idx{1};
    int write_idx{1};
    int socket_idx{1};
    int summary_impl{0};
    int extract_impl{0};
    int process_impl{0};
    int timer_idx{0};
};

static int& coord(Theta& t, Dim d){
    switch(d){
        case Dim::ReadChunk:return t.read_idx; case Dim::WriteChunk:return t.write_idx;
        case Dim::SocketChunk:return t.socket_idx; case Dim::SummaryImpl:return t.summary_impl;
        case Dim::ExtractImpl:return t.extract_impl; case Dim::ProcessImpl:return t.process_impl;
        case Dim::TimerQuantum:return t.timer_idx;
    }
    return t.read_idx;
}
static int max_index(Dim d){
    switch(d){
        case Dim::ReadChunk: case Dim::WriteChunk: case Dim::SocketChunk:return static_cast<int>(CHUNKS.size()-1);
        case Dim::TimerQuantum:return static_cast<int>(TIMER_US.size()-1);
        case Dim::SummaryImpl: case Dim::ExtractImpl: case Dim::ProcessImpl:return 1;
    }
    return 0;
}
static std::string theta_string(const Theta&t){
    std::ostringstream o;o<<"r="<<CHUNKS[t.read_idx]<<";w="<<CHUNKS[t.write_idx]<<";s="<<CHUNKS[t.socket_idx]
      <<";sum="<<t.summary_impl<<";ext="<<t.extract_impl<<";proc="<<t.process_impl<<";timer="<<TIMER_US[t.timer_idx];return o.str();
}

struct BigTcpServer {
    int fd{-1}; uint16_t port{}; std::atomic<bool> stop{false}; std::thread th; std::string payload;
    BigTcpServer(){
        std::ostringstream p;p<<"<html><body><h1>Local DRM benchmark</h1><p>";
        for(int i=0;i<12000;i++)p<<"token"<<(i%97)<<" optimization repeated local task ";
        p<<"</p></body></html>";payload=p.str();
        fd=::socket(AF_INET,SOCK_STREAM,0);if(fd<0)throw std::runtime_error("socket");int one=1;setsockopt(fd,SOL_SOCKET,SO_REUSEADDR,&one,sizeof(one));
        sockaddr_in a{};a.sin_family=AF_INET;a.sin_addr.s_addr=htonl(INADDR_LOOPBACK);a.sin_port=0;
        if(bind(fd,reinterpret_cast<sockaddr*>(&a),sizeof(a))<0)throw std::runtime_error("bind");
        socklen_t n=sizeof(a);getsockname(fd,reinterpret_cast<sockaddr*>(&a),&n);port=ntohs(a.sin_port);listen(fd,16);th=std::thread([this]{loop();});
    }
    void loop(){while(!stop){pollfd pfd{fd,POLLIN,0};int r=poll(&pfd,1,20);if(r<=0)continue;int c=accept(fd,nullptr,nullptr);if(c<0)continue;char b[2048];(void)read(c,b,sizeof(b));std::ostringstream resp;resp<<"HTTP/1.1 200 OK\r\nContent-Length: "<<payload.size()<<"\r\nConnection: close\r\n\r\n";auto h=resp.str();write(c,h.data(),h.size());size_t off=0;while(off<payload.size()){ssize_t n=write(c,payload.data()+off,payload.size()-off);if(n<=0)break;off+=static_cast<size_t>(n);}close(c);} }
    ~BigTcpServer(){stop=true;if(th.joinable())th.join();if(fd>=0)close(fd);}
};

struct ExecSample { bool ok{false}; double wall_ms{},cpu_ms{}; std::string value; };

struct TunedExecutor {
    fs::path work; uint16_t port; size_t commits{},process_spawns{},tcp_requests{},timer_events{}; std::map<std::string,size_t> root_counts;
    TunedExecutor(fs::path w,uint16_t p):work(std::move(w)),port(p){}
    void roots(const std::string& cap){for(const auto&r:CAP_ROOT.at(cap))root_counts[r]++;}

    static std::string read_file(const fs::path&p,size_t chunk){int fd=open(p.c_str(),O_RDONLY);if(fd<0)throw std::runtime_error("open read");std::string out;std::vector<char>b(chunk);for(;;){ssize_t n=read(fd,b.data(),b.size());if(n<0){close(fd);throw std::runtime_error("read");}if(n==0)break;out.append(b.data(),static_cast<size_t>(n));}close(fd);return out;}
    static void write_file_atomic(const fs::path&p,const std::string&data,size_t chunk){fs::create_directories(p.parent_path());auto tmp=p;tmp+=".candidate";int fd=open(tmp.c_str(),O_WRONLY|O_CREAT|O_TRUNC,0644);if(fd<0)throw std::runtime_error("open write");size_t off=0;while(off<data.size()){size_t want=std::min(chunk,data.size()-off);ssize_t n=write(fd,data.data()+off,want);if(n<=0){close(fd);throw std::runtime_error("write");}off+=static_cast<size_t>(n);}fsync(fd);close(fd);fs::rename(tmp,p);}
    std::string http_get(const std::string& path,size_t chunk){int s=socket(AF_INET,SOCK_STREAM,0);if(s<0)throw std::runtime_error("socket");sockaddr_in a{};a.sin_family=AF_INET;a.sin_port=htons(port);inet_pton(AF_INET,"127.0.0.1",&a.sin_addr);if(connect(s,reinterpret_cast<sockaddr*>(&a),sizeof(a))<0){close(s);throw std::runtime_error("connect");}std::string q="GET "+path+" HTTP/1.1\r\nHost:127.0.0.1\r\nConnection:close\r\n\r\n";write(s,q.data(),q.size());std::string r;std::vector<char>b(chunk);for(;;){ssize_t n=read(s,b.data(),b.size());if(n<=0)break;r.append(b.data(),static_cast<size_t>(n));}close(s);tcp_requests++;auto pos=r.find("\r\n\r\n");return pos==std::string::npos?r:r.substr(pos+4);}
    static std::string extract_slow(std::string s){return LiveExecutor::extract(std::move(s));}
    static std::string extract_fast(const std::string&s){std::string o;o.reserve(s.size());bool tag=false;for(char c:s){if(c=='<'){tag=true;continue;}if(c=='>'){tag=false;o.push_back(' ');continue;}if(!tag)o.push_back(c);}std::string r;r.reserve(o.size());bool in_ws=true;for(char c:o){bool ws=std::isspace(static_cast<unsigned char>(c));if(ws){if(!in_ws)r.push_back(' ');}else r.push_back(c);in_ws=ws;}if(!r.empty()&&r.back()==' ')r.pop_back();return r;}
    static std::string summarize_fast(const std::string&s){size_t words=0;std::vector<std::string>head;head.reserve(10);size_t i=0;while(i<s.size()){while(i<s.size()&&std::isspace(static_cast<unsigned char>(s[i])))i++;if(i==s.size())break;size_t j=i;while(j<s.size()&&!std::isspace(static_cast<unsigned char>(s[j])))j++;words++;if(head.size()<10)head.emplace_back(s.substr(i,j-i));i=j;}std::ostringstream o;o<<"words="<<words<<" head=";for(size_t k=0;k<head.size();k++){if(k)o<<' ';o<<head[k];}return o.str();}
    std::string run_hash_fork(const fs::path&p,size_t chunk){int fds[2];if(pipe(fds)<0)throw std::runtime_error("pipe");pid_t pid=fork();if(pid<0)throw std::runtime_error("fork");if(pid==0){dup2(fds[1],STDOUT_FILENO);close(fds[0]);close(fds[1]);execlp("sha256sum","sha256sum",p.c_str(),static_cast<char*>(nullptr));_exit(127);}close(fds[1]);std::string out;std::vector<char>b(chunk);for(;;){ssize_t n=read(fds[0],b.data(),b.size());if(n<=0)break;out.append(b.data(),static_cast<size_t>(n));}close(fds[0]);int st=0;waitpid(pid,&st,0);process_spawns++;if(!WIFEXITED(st)||WEXITSTATUS(st)!=0)throw std::runtime_error("child failed");return out;}
    std::string run_hash_spawn(const fs::path&p,size_t chunk){int fds[2];if(pipe(fds)<0)throw std::runtime_error("pipe");posix_spawn_file_actions_t acts;posix_spawn_file_actions_init(&acts);posix_spawn_file_actions_adddup2(&acts,fds[1],STDOUT_FILENO);posix_spawn_file_actions_addclose(&acts,fds[0]);posix_spawn_file_actions_addclose(&acts,fds[1]);std::string ps=p.string();char* argv[]={const_cast<char*>("sha256sum"),ps.data(),nullptr};pid_t pid{};int rc=posix_spawnp(&pid,"sha256sum",&acts,nullptr,argv,environ);posix_spawn_file_actions_destroy(&acts);close(fds[1]);if(rc!=0){close(fds[0]);throw std::runtime_error("posix_spawn");}std::string out;std::vector<char>b(chunk);for(;;){ssize_t n=read(fds[0],b.data(),b.size());if(n<=0)break;out.append(b.data(),static_cast<size_t>(n));}close(fds[0]);int st=0;waitpid(pid,&st,0);process_spawns++;if(!WIFEXITED(st)||WEXITSTATUS(st)!=0)throw std::runtime_error("spawn child failed");return out;}

    ExecSample execute(const Episode&ep,const Theta&t){auto u0=usage();auto start=Clock::now();std::string data;bool ok=false;try{for(const auto&cap:ep.ops){roots(cap);if(cap=="fs.read")data=read_file(work/ep.source,CHUNKS[t.read_idx]);else if(cap=="http.request")data=http_get(ep.url_path,CHUNKS[t.socket_idx]);else if(cap=="process.run")data=t.process_impl?run_hash_spawn(work/ep.source,CHUNKS[t.read_idx]):run_hash_fork(work/ep.source,CHUNKS[t.read_idx]);else if(cap=="proc.observe")data=read_file("/proc/self/status",CHUNKS[t.read_idx]);else if(cap=="timer.observe"){auto until=Clock::now()+std::chrono::milliseconds(2);while(Clock::now()<until)std::this_thread::sleep_for(std::chrono::microseconds(TIMER_US[t.timer_idx]));data="timer-fired";timer_events++;}else if(cap=="transform.extract")data=t.extract_impl?extract_fast(data):extract_slow(data);else if(cap=="transform.summarize")data=t.summary_impl?summarize_fast(data):LiveExecutor::summarize(data);else if(cap=="fs.write"){write_file_atomic(work/ep.output,data,CHUNKS[t.write_idx]);commits++;}else throw std::runtime_error("unsupported optimization cap: "+cap);}ok=true;}catch(...){ok=false;}auto end=Clock::now();auto u1=usage();return{ok,std::chrono::duration<double,std::milli>(end-start).count(),(u1.u-u0.u+u1.s-u0.s)*1000.0,data};}
};

static std::string family_key(const Episode&e){std::string k;for(const auto&o:e.ops){if(!k.empty())k+='|';k+=o;}return k;}
static std::vector<Dim> active_dims(const Episode&e){std::vector<Dim>d;auto has=[&](const char*x){return std::find(e.ops.begin(),e.ops.end(),x)!=e.ops.end();};if(has("fs.read")||has("proc.observe"))d.push_back(Dim::ReadChunk);if(has("fs.write"))d.push_back(Dim::WriteChunk);if(has("http.request"))d.push_back(Dim::SocketChunk);if(has("transform.summarize"))d.push_back(Dim::SummaryImpl);if(has("transform.extract"))d.push_back(Dim::ExtractImpl);if(has("process.run"))d.push_back(Dim::ProcessImpl);if(has("timer.observe"))d.push_back(Dim::TimerQuantum);if(d.size()>3)d.resize(3);return d;}
static double median(std::vector<double>x){std::sort(x.begin(),x.end());if(x.empty())return 0;return x.size()%2?x[x.size()/2]:(x[x.size()/2-1]+x[x.size()/2])/2;}

struct ProbeResult {bool valid{false};double current_cost{},candidate_cost{},delta{},candidate_wall{},candidate_cpu{};int wins{};};
struct RuntimeDescent {
    Theta theta; std::vector<Dim> dims; std::map<Dim,int> step; std::map<Dim,bool> stable; size_t cursor{},probes{},commits{};bool converged{};size_t convergence_episode{};double best_cost{INFINITY};
    explicit RuntimeDescent(std::vector<Dim>d):dims(std::move(d)){for(auto x:dims){step[x]=(max_index(x)<=1?1:2);stable[x]=false;}}
    static double cost(const ExecSample&s){return s.wall_ms+0.10*s.cpu_ms;}
    static bool materially_better(const ProbeResult&p,int pairs){double required=std::max(0.010,0.020*p.current_cost);return p.valid && p.wins >= (pairs+1)/2 && p.delta < -required;}
    ProbeResult compare(TunedExecutor&ex,const Episode&e,const Theta&cand,int pairs=3){std::vector<double>a,b;std::vector<double>bw,bc;bool valid=true;int wins=0;for(int i=0;i<pairs;i++){bool candidate_first=(i%2)==1;ExecSample x=candidate_first?ex.execute(e,cand):ex.execute(e,theta);ExecSample y=candidate_first?ex.execute(e,theta):ex.execute(e,cand);ExecSample cur=candidate_first?y:x;ExecSample can=candidate_first?x:y;if(!cur.ok||!can.ok||cur.value!=can.value){valid=false;break;}double cc=cost(cur),kc=cost(can);a.push_back(cc);b.push_back(kc);bw.push_back(can.wall_ms);bc.push_back(can.cpu_ms);if(kc<cc)wins++;}probes+=static_cast<size_t>(pairs*2);return{valid,median(a),median(b),median(b)-median(a),median(bw),median(bc),wins};}
    bool optimize_once(TunedExecutor&ex,const Episode&e,size_t episode,std::ofstream&log){if(converged||dims.empty())return false;Dim d=dims[cursor%dims.size()];cursor=(cursor+1)%dims.size();int cur=coord(theta,d);int st=step[d];std::vector<std::pair<Theta,ProbeResult>> candidates;for(int sign:{-1,1}){int nv=cur+sign*st;if(nv<0||nv>max_index(d))continue;Theta c=theta;coord(c,d)=nv;auto pr=compare(ex,e,c,3);if(pr.valid)candidates.push_back({c,pr});}
        bool improved=false;Theta best=theta;ProbeResult bp;for(const auto&x:candidates){if(materially_better(x.second,3)&&(!improved||x.second.candidate_cost<bp.candidate_cost)){improved=true;best=x.first;bp=x.second;}}
        if(improved){auto before=theta_string(theta);theta=best;commits++;for(auto x:dims)stable[x]=false;best_cost=std::min(best_cost,bp.candidate_cost);log<<episode<<','<<dim_name(d)<<",commit,"<<esc(before)<<','<<esc(theta_string(theta))<<','<<st<<','<<bp.current_cost<<','<<bp.candidate_cost<<','<<bp.delta<<"\n";return true;}
        if(st>1){step[d]=std::max(1,st/2);stable[d]=false;log<<episode<<','<<dim_name(d)<<",shrink,"<<esc(theta_string(theta))<<','<<esc(theta_string(theta))<<','<<st<<",0,0,0\n";}
        else {stable[d]=true;log<<episode<<','<<dim_name(d)<<",stable,"<<esc(theta_string(theta))<<','<<esc(theta_string(theta))<<",1,0,0,0\n";}
        bool all=true;for(auto x:dims)all=all&&stable[x];if(all){converged=true;convergence_episode=episode;log<<episode<<",ALL,converged,"<<esc(theta_string(theta))<<','<<esc(theta_string(theta))<<",0,0,0,0\n";}return false;}
    bool certify(TunedExecutor&ex,const Episode&e,std::ostream&log){
        constexpr int pairs=5;for(int pass=0;pass<8;pass++){bool found=false;Theta best=theta;ProbeResult bp;Dim bd=dims.empty()?Dim::ReadChunk:dims.front();for(Dim d:dims){int cur=coord(theta,d);for(int sign:{-1,1}){int nv=cur+sign;if(nv<0||nv>max_index(d))continue;Theta c=theta;coord(c,d)=nv;auto pr=compare(ex,e,c,pairs);log<<dim_name(d)<<','<<coord(theta,d)<<','<<nv<<','<<pr.current_cost<<','<<pr.candidate_cost<<','<<pr.delta<<','<<pr.wins<<','<<pr.valid<<"\n";if(materially_better(pr,pairs)&&(!found||pr.candidate_cost<bp.candidate_cost)){found=true;best=c;bp=pr;bd=d;}}}if(!found)return true;theta=best;commits++;for(auto x:dims)stable[x]=false;converged=false;log<<"COMMIT,"<<dim_name(bd)<<",-1,-1,"<<bp.current_cost<<','<<bp.candidate_cost<<','<<bp.delta<<','<<bp.wins<<",1\n";}return false;}
};

struct FamilyState {std::string name;Episode exemplar;RuntimeDescent opt;size_t stable_streak{},episodes{};double optimizer_wall_ms{};std::vector<double>walls;FamilyState(std::string n,Episode e,std::vector<Dim>d):name(std::move(n)),exemplar(std::move(e)),opt(std::move(d)){} };

static void make_large_fixtures(const fs::path&w){fs::remove_all(w);fs::create_directories(w/"inputs");fs::create_directories(w/"outputs");for(int f=0;f<8;f++){std::ofstream o(w/"inputs"/("large_"+std::to_string(f)+".txt"));o<<"<html><body>\n";for(int i=0;i<50000;i++)o<<"row"<<i<<" value"<<(i%97)<<" repeated optimization local DRM task workload\n";o<<"</body></html>\n";}}

static std::vector<Episode> opt_workload(){std::vector<Episode>v;size_t i=0;auto addrep=[&](std::string task,Seq ops,int n){for(int r=0;r<n;r++){Episode e; e.idx=++i;e.task=task;e.phase="runtime_descent";e.ops=ops;e.source="inputs/large_"+std::to_string(r%8)+".txt";e.output="outputs/"+task+".txt";e.url_path="/large";v.push_back(std::move(e));}};
    addrep("opt_file",seq({"fs.read","transform.extract","transform.summarize","fs.write"}),45);
    addrep("opt_http",seq({"http.request","transform.extract","transform.summarize","fs.write"}),45);
    addrep("opt_hash",seq({"process.run","transform.summarize","fs.write"}),35);
    addrep("opt_proc",seq({"proc.observe","transform.extract","transform.summarize","fs.write"}),35);
    addrep("opt_timer",seq({"timer.observe","transform.summarize","fs.write"}),35);
    return v;}

static int run_optimizer(const fs::path&out){fs::create_directories(out);auto work=out/"workspace";make_large_fixtures(work);BigTcpServer tcp;TunedExecutor ex(work,tcp.port);DrmPlanner planner;
    // Carry the base developmental history/vocabulary forward into runtime descent.
    for(const auto&be:workload())(void)planner.plan(be);
    auto eps=opt_workload();std::map<std::string,std::unique_ptr<FamilyState>> families;
    std::ofstream trace(out/"runtime_trace.csv");trace<<"episode,task,family,success,semantic,structural_change,stable_streak,runtime_state,wall_ms,cpu_ms,theta,derived,uniform\n";
    std::ofstream optlog(out/"optimization_trace.csv");optlog<<"episode,coordinate,event,theta_before,theta_after,step,current_cost,candidate_cost,delta\n";
    size_t okcount=0,total_sem=0;double prod_wall=0;long peak=0;
    for(const auto&e:eps){auto pm=planner.plan(e);std::string fk=family_key(e);if(!families.contains(fk))families[fk]=std::make_unique<FamilyState>(e.task,e,active_dims(e));auto&f=*families[fk];f.episodes++;if(pm.semantic==1&&pm.structural_change==0&&pm.local_repair==0&&pm.recovery==0)f.stable_streak++;else f.stable_streak=0;auto s=ex.execute(e,f.opt.theta);f.walls.push_back(s.wall_ms);okcount+=s.ok;total_sem+=pm.semantic;prod_wall+=s.wall_ms;peak=std::max(peak,rss_kb());std::string state=f.opt.converged?"CONVERGED":(f.stable_streak>=3?"OPTIMIZING":"DEVELOPING");trace<<e.idx<<','<<e.task<<','<<std::hash<std::string>{}(fk)<<','<<s.ok<<','<<pm.semantic<<','<<pm.structural_change<<','<<f.stable_streak<<','<<state<<','<<s.wall_ms<<','<<s.cpu_ms<<','<<esc(theta_string(f.opt.theta))<<','<<pm.derived<<','<<pm.uniform<<"\n";if(f.stable_streak>=3&&!f.opt.converged){auto q0=Clock::now();f.opt.optimize_once(ex,e,e.idx,optlog);f.optimizer_wall_ms+=std::chrono::duration<double,std::milli>(Clock::now()-q0).count();}}

    std::ofstream cert(out/"local_optimum_certificate.csv");cert<<"family,coordinate,current_index,candidate_index,current_cost,candidate_cost,delta,wins,valid\n";
    size_t certified=0;for(auto&[_,ptr]:families){auto&f=*ptr;auto c0=Clock::now();std::ostringstream local;bool c=f.opt.certify(ex,f.exemplar,local);f.optimizer_wall_ms+=std::chrono::duration<double,std::milli>(Clock::now()-c0).count();std::istringstream in(local.str());std::string line;while(std::getline(in,line)){if(!line.empty())cert<<f.name<<','<<line<<"\n";}if(c){f.opt.converged=true;certified++;}}

    std::ofstream val(out/"validation_pairs.csv");val<<"family,default_wall_median_ms,optimized_wall_median_ms,default_cost_median,optimized_cost_median,speedup,saving_ms,optimizer_wall_ms,break_even_episodes,output_equivalent,theta\n";
    std::ofstream fsout(out/"family_summary.csv");fsout<<"family,episodes,converged,convergence_episode,dimensions,optimizer_probes,optimizer_commits,preopt_median_ms,last10_median_ms,observed_speedup,theta\n";
    size_t converged=0,probes=0,optcommits=0;double validation_speed_sum=0;size_t validation_improved=0;Theta default_theta;
    for(auto&[_,ptr]:families){auto&f=*ptr;std::vector<double>pre=f.walls; if(pre.size()>3)pre.resize(3);std::vector<double>last=f.walls;if(last.size()>10)last=std::vector<double>(last.end()-10,last.end());double premed=median(pre),lm=median(last);double observed=lm>0?premed/lm:0;converged+=f.opt.converged;probes+=f.opt.probes;optcommits+=f.opt.commits;std::string ds;for(size_t k=0;k<f.opt.dims.size();k++){if(k)ds+='|';ds+=dim_name(f.opt.dims[k]);}fsout<<f.name<<','<<f.episodes<<','<<f.opt.converged<<','<<f.opt.convergence_episode<<','<<esc(ds)<<','<<f.opt.probes<<','<<f.opt.commits<<','<<premed<<','<<lm<<','<<observed<<','<<esc(theta_string(f.opt.theta))<<"\n";
        std::vector<double>dw,ow,dc,oc;bool eq=true;constexpr int pairs=9;for(int i=0;i<pairs;i++){bool opt_first=(i%2)==1;auto a=opt_first?ex.execute(f.exemplar,f.opt.theta):ex.execute(f.exemplar,default_theta);auto b=opt_first?ex.execute(f.exemplar,default_theta):ex.execute(f.exemplar,f.opt.theta);auto def=opt_first?b:a;auto opt=opt_first?a:b;if(!def.ok||!opt.ok||def.value!=opt.value)eq=false;dw.push_back(def.wall_ms);ow.push_back(opt.wall_ms);dc.push_back(RuntimeDescent::cost(def));oc.push_back(RuntimeDescent::cost(opt));}double dm=median(dw),om=median(ow),dcm=median(dc),ocm=median(oc),saving=dm-om,speed=om>0?dm/om:0;double be=saving>0?f.optimizer_wall_ms/saving:INFINITY;if(saving>0)validation_improved++;validation_speed_sum+=speed;val<<f.name<<','<<dm<<','<<om<<','<<dcm<<','<<ocm<<','<<speed<<','<<saving<<','<<f.optimizer_wall_ms<<','<<(std::isfinite(be)?be:-1)<<','<<eq<<','<<esc(theta_string(f.opt.theta))<<"\n";}
    bool uniform=planner.vocab.audit();std::ofstream js(out/"runtime_summary.json");js<<std::fixed<<std::setprecision(6)<<"{\n  \"episodes\": "<<eps.size()<<",\n  \"success_rate\": "<<double(okcount)/eps.size()<<",\n  \"semantic_total\": "<<total_sem<<",\n  \"families\": "<<families.size()<<",\n  \"families_converged\": "<<converged<<",\n  \"families_certified_local\": "<<certified<<",\n  \"families_validation_faster\": "<<validation_improved<<",\n  \"optimizer_probes\": "<<probes<<",\n  \"optimizer_commits\": "<<optcommits<<",\n  \"mean_validation_speedup\": "<<validation_speed_sum/families.size()<<",\n  \"production_wall_ms\": "<<prod_wall<<",\n  \"peak_rss_kb\": "<<peak<<",\n  \"derived_final\": "<<planner.vocab.derived.size()<<",\n  \"uniform_vocabulary\": "<<(uniform?"true":"false")<<",\n  \"root_vocabulary\": [\"OBSERVE\", \"DERIVE\", \"COMMIT\"]\n}\n";
    std::cout<<"runtime_episodes="<<eps.size()<<" success="<<okcount<<" families="<<families.size()<<" converged="<<converged<<" certified="<<certified<<" probes="<<probes<<" opt_commits="<<optcommits<<" uniform="<<uniform<<" validation_faster="<<validation_improved<<"\n";return(okcount==eps.size()&&uniform&&certified==families.size())?0:3;}


struct OnlineDescent {
    Theta theta;std::vector<Dim>dims;std::map<Dim,int>step;std::map<Dim,int>stable_passes;size_t cursor{};bool converged{};size_t convergence_episode{};size_t candidate_episodes{},commits{};double bookkeeping_ms{},exploration_regret{};std::vector<double>baseline;std::vector<std::pair<Theta,std::pair<Dim,int>>> pending;std::vector<std::tuple<Theta,Dim,int,double>> observed;
    explicit OnlineDescent(std::vector<Dim>d):dims(std::move(d)){for(auto x:dims){step[x]=(max_index(x)<=1?1:2);stable_passes[x]=0;}}
    double baseline_cost()const{return baseline.empty()?INFINITY:median(baseline);}
    std::pair<Theta,bool> choose(){auto t0=Clock::now();if(converged||dims.empty()||baseline.size()<3){bookkeeping_ms+=std::chrono::duration<double,std::milli>(Clock::now()-t0).count();return{theta,false};}if(pending.empty()){Dim d=dims[cursor%dims.size()];int cur=coord(theta,d),st=step[d];for(int sign:{-1,1}){int nv=cur+sign*st;if(nv<0||nv>max_index(d))continue;Theta c=theta;coord(c,d)=nv;pending.push_back({c,{d,nv}});}if(pending.empty()){stable_passes[d]++;cursor=(cursor+1)%dims.size();}}
        if(pending.empty()){bookkeeping_ms+=std::chrono::duration<double,std::milli>(Clock::now()-t0).count();return{theta,false};}Theta c=pending.front().first;bookkeeping_ms+=std::chrono::duration<double,std::milli>(Clock::now()-t0).count();return{c,true};}
    void observe(const Theta&used,bool candidate,const ExecSample&s,size_t episode,std::ofstream&log){auto t0=Clock::now();double c=RuntimeDescent::cost(s);if(!candidate){baseline.push_back(c);if(baseline.size()>5)baseline.erase(baseline.begin());bookkeeping_ms+=std::chrono::duration<double,std::milli>(Clock::now()-t0).count();return;}candidate_episodes++;double base=baseline_cost();if(std::isfinite(base)&&c>base)exploration_regret+=c-base;auto meta=pending.front().second;pending.erase(pending.begin());observed.push_back({used,meta.first,meta.second,c});log<<episode<<','<<dim_name(meta.first)<<",probe,"<<base<<','<<c<<','<<(c-base)<<','<<esc(theta_string(used))<<"\n";if(!pending.empty()){bookkeeping_ms+=std::chrono::duration<double,std::milli>(Clock::now()-t0).count();return;}
        Dim d=meta.first;double required=std::max(0.050,(step[d]>1?0.03:0.02)*base);bool found=false;Theta best=theta;double bestc=base;for(const auto&o:observed){if(std::get<1>(o)!=d)continue;double oc=std::get<3>(o);if(oc<base-required&&(!found||oc<bestc)){found=true;best=std::get<0>(o);bestc=oc;}}
        if(found){auto before=theta_string(theta);theta=best;commits++;baseline.clear();baseline.push_back(bestc);for(auto x:dims)stable_passes[x]=0;log<<episode<<','<<dim_name(d)<<",commit,"<<base<<','<<bestc<<','<<(bestc-base)<<','<<esc(before+" -> "+theta_string(theta))<<"\n";}
        else {if(step[d]>1){step[d]=std::max(1,step[d]/2);stable_passes[d]=0;log<<episode<<','<<dim_name(d)<<",shrink,"<<base<<','<<base<<",0,"<<esc(theta_string(theta))<<"\n";}else{stable_passes[d]++;log<<episode<<','<<dim_name(d)<<",stable,"<<base<<','<<base<<",0,"<<esc(theta_string(theta))<<"\n";}cursor=(cursor+1)%dims.size();}
        observed.clear();bool all=!dims.empty();for(auto x:dims)all=all&&(stable_passes[x]>=2);if(all){converged=true;convergence_episode=episode;log<<episode<<",ALL,converged,"<<base<<','<<base<<",0,"<<esc(theta_string(theta))<<"\n";}bookkeeping_ms+=std::chrono::duration<double,std::milli>(Clock::now()-t0).count();}
};

struct OnlineFamily {std::string name;Episode exemplar;OnlineDescent opt;size_t stable_streak{},episodes{};std::vector<double>walls;std::string reference;OnlineFamily(std::string n,Episode e,std::vector<Dim>d):name(std::move(n)),exemplar(std::move(e)),opt(std::move(d)){} };

static std::vector<Episode> online_workload(){std::vector<Episode>v;size_t i=0;auto addrep=[&](std::string task,Seq ops,int n){for(int r=0;r<n;r++){Episode e;e.idx=++i;e.task=task;e.phase="online_descent";e.ops=ops;e.source="inputs/large_"+std::to_string(r%8)+".txt";e.output="outputs/"+task+".txt";e.url_path="/large";v.push_back(std::move(e));}};addrep("online_file",seq({"fs.read","transform.extract","transform.summarize","fs.write"}),60);addrep("online_http",seq({"http.request","transform.extract","transform.summarize","fs.write"}),60);addrep("online_hash",seq({"process.run","transform.summarize","fs.write"}),55);addrep("online_proc",seq({"proc.observe","transform.extract","transform.summarize","fs.write"}),55);addrep("online_timer",seq({"timer.observe","transform.summarize","fs.write"}),80);return v;}

static int run_optimizer_online(const fs::path& out) {
    fs::create_directories(out);
    auto work = out / "workspace";
    make_large_fixtures(work);
    BigTcpServer tcp;
    TunedExecutor ex(work, tcp.port);
    DrmPlanner planner;

    // Preserve the existing DRM developmental lineage before runtime descent.
    for (const auto& be : workload()) (void)planner.plan(be);

    auto eps = online_workload();
    std::map<std::string, std::unique_ptr<OnlineFamily>> families;
    std::ofstream tr(out / "online_trace.csv");
    tr << "episode,task,success,semantic,structural_change,state,candidate,wall_ms,cpu_ms,theta,derived,uniform\n";
    std::ofstream lg(out / "online_descent_trace.csv");
    lg << "episode,coordinate,event,baseline_cost,observed_cost,delta,theta\n";

    size_t okcount = 0, total_sem = 0;
    long peak = 0;

    for (const auto& e : eps) {
        auto pm = planner.plan(e);
        std::string fk = family_key(e);
        if (!families.contains(fk)) {
            families[fk] = std::make_unique<OnlineFamily>(e.task, e, active_dims(e));
        }
        auto& f = *families[fk];
        f.episodes++;
        if (pm.semantic == 1 && pm.structural_change == 0 && pm.local_repair == 0 && pm.recovery == 0) {
            f.stable_streak++;
        } else {
            f.stable_streak = 0;
        }

        Theta used = f.opt.theta;
        bool candidate = false;
        if (f.stable_streak >= 3) {
            auto choice = f.opt.choose();
            used = choice.first;
            candidate = choice.second;
        }

        auto sample = ex.execute(e, used);
        if (f.reference.empty()) f.reference = sample.value;
        if (f.stable_streak >= 3) f.opt.observe(used, candidate, sample, e.idx, lg);

        f.walls.push_back(sample.wall_ms);
        okcount += sample.ok;
        total_sem += pm.semantic;
        peak = std::max(peak, rss_kb());
        std::string state = f.opt.converged ? "CONVERGED" : (f.stable_streak >= 3 ? "DESCENT" : "DEVELOPING");
        tr << e.idx << ',' << e.task << ',' << sample.ok << ',' << pm.semantic << ','
           << pm.structural_change << ',' << state << ',' << candidate << ',' << sample.wall_ms << ','
           << sample.cpu_ms << ',' << esc(theta_string(used)) << ',' << pm.derived << ',' << pm.uniform << "\n";
    }

    // Idle-time close-convergence certification. This is deliberately separate from
    // production optimizer cost. It first anchors against the known default and then
    // performs strict adjacent-neighbor refinement until no robust local improvement remains.
    Theta default_theta;
    size_t anchor_rollbacks = 0, idle_refine_commits = 0, certified_local = 0;
    double idle_test_wall_ms = 0.0;
    std::ofstream cert(out / "online_local_certificate.csv");
    cert << "family,pass,event,coordinate,current_index,candidate_index,current_cost,candidate_cost,delta,wins,theta\n";
    std::ofstream anchorlog(out / "baseline_anchor.csv");
    anchorlog << "family,pass,default_median_ms,candidate_median_ms,speedup,saving_ms,action,theta\n";

    auto paired_wall = [&](OnlineFamily& f, const Theta& a, const Theta& b, int pairs) {
        std::vector<double> aw, bw;
        bool eq = true;
        for (int i = 0; i < pairs; ++i) {
            bool bfirst = (i % 2) == 1;
            auto x = bfirst ? ex.execute(f.exemplar, b) : ex.execute(f.exemplar, a);
            auto y = bfirst ? ex.execute(f.exemplar, a) : ex.execute(f.exemplar, b);
            auto as = bfirst ? y : x;
            auto bs = bfirst ? x : y;
            if (!as.ok || !bs.ok || as.value != bs.value) eq = false;
            aw.push_back(as.wall_ms);
            bw.push_back(bs.wall_ms);
        }
        return std::tuple<double,double,bool>{median(aw), median(bw), eq};
    };

    for (auto& [_, ptr] : families) {
        auto& f = *ptr;
        auto idle0 = Clock::now();
        bool certified = false;

        for (int pass = 0; pass < 8 && !certified; ++pass) {
            // Baseline anchor: never knowingly remain in a basin that is not materially
            // faster than the last verified default configuration.
            auto [dm, cm, eq_anchor] = paired_wall(f, default_theta, f.opt.theta, 7);
            double saving = dm - cm;
            double speed = cm > 0 ? dm / cm : 0;
            bool is_default = theta_string(f.opt.theta) == theta_string(default_theta);
            bool keep_candidate = eq_anchor && (is_default || (saving >= 0.050 && speed >= 1.02));
            if (!keep_candidate) {
                f.opt.theta = default_theta;
                anchor_rollbacks++;
                anchorlog << f.name << ',' << pass << ',' << dm << ',' << cm << ',' << speed << ','
                          << saving << ",rollback," << esc(theta_string(f.opt.theta)) << "\n";
            } else {
                anchorlog << f.name << ',' << pass << ',' << dm << ',' << cm << ',' << speed << ','
                          << saving << ",keep," << esc(theta_string(f.opt.theta)) << "\n";
            }

            // Strict epsilon-local search at one discrete coordinate step.
            bool found = false;
            Theta best = f.opt.theta;
            double best_cost = INFINITY;
            Dim best_dim = f.opt.dims.empty() ? Dim::ReadChunk : f.opt.dims.front();
            int best_nv = -1;
            ProbeResult best_pr;
            RuntimeDescent verifier(f.opt.dims);
            verifier.theta = f.opt.theta;

            for (Dim d : f.opt.dims) {
                int cur = coord(f.opt.theta, d);
                for (int sign : {-1, 1}) {
                    int nv = cur + sign;
                    if (nv < 0 || nv > max_index(d)) continue;
                    Theta cand = f.opt.theta;
                    coord(cand, d) = nv;
                    auto pr = verifier.compare(ex, f.exemplar, cand, 5);
                    double required = std::max(0.050, 0.030 * pr.current_cost);
                    bool better = pr.valid && pr.wins >= 4 && pr.delta < -required;
                    cert << f.name << ',' << pass << ",probe," << dim_name(d) << ',' << cur << ',' << nv << ','
                         << pr.current_cost << ',' << pr.candidate_cost << ',' << pr.delta << ',' << pr.wins << ','
                         << esc(theta_string(f.opt.theta)) << "\n";
                    if (better && pr.candidate_cost < best_cost) {
                        found = true;
                        best = cand;
                        best_cost = pr.candidate_cost;
                        best_dim = d;
                        best_nv = nv;
                        best_pr = pr;
                    }
                }
            }

            if (found) {
                f.opt.theta = best;
                idle_refine_commits++;
                cert << f.name << ',' << pass << ",idle_commit," << dim_name(best_dim) << ",-1," << best_nv << ','
                     << best_pr.current_cost << ',' << best_pr.candidate_cost << ',' << best_pr.delta << ','
                     << best_pr.wins << ',' << esc(theta_string(f.opt.theta)) << "\n";
                continue;
            }

            // One more anchor check after local stability. If default is better, roll back
            // and repeat the neighborhood search around default. Otherwise certify.
            auto [fdm, fcm, eq_final] = paired_wall(f, default_theta, f.opt.theta, 9);
            double fsaving = fdm - fcm;
            double fspeed = fcm > 0 ? fdm / fcm : 0;
            bool final_is_default = theta_string(f.opt.theta) == theta_string(default_theta);
            bool final_ok = eq_final && (final_is_default || (fsaving >= 0.050 && fspeed >= 1.02));
            if (!final_ok) {
                f.opt.theta = default_theta;
                anchor_rollbacks++;
                anchorlog << f.name << ',' << pass << ',' << fdm << ',' << fcm << ',' << fspeed << ','
                          << fsaving << ",final_rollback," << esc(theta_string(f.opt.theta)) << "\n";
                continue;
            }

            certified = true;
            certified_local++;
            f.opt.converged = true;
            cert << f.name << ',' << pass << ",certified,ALL,-1,-1,0,0,0,0,"
                 << esc(theta_string(f.opt.theta)) << "\n";
        }

        idle_test_wall_ms += std::chrono::duration<double,std::milli>(Clock::now() - idle0).count();
    }

    // Independent final A/B validation against the default anchor.
    std::ofstream val(out / "validation_pairs.csv");
    val << "family,default_wall_median_ms,optimized_wall_median_ms,speedup,saving_ms,exploration_regret_ms,break_even_episodes,output_equivalent,theta\n";
    std::ofstream fsout(out / "family_summary.csv");
    fsout << "family,episodes,converged,convergence_episode,dimensions,candidate_episodes,commits,bookkeeping_ms,exploration_regret_ms,preopt_median_ms,last10_median_ms,observed_speedup,theta\n";

    size_t conv = 0, faster = 0, total_candidates = 0, total_commits = 0;
    double speed_sum = 0, book = 0, regret = 0;

    for (auto& [_, ptr] : families) {
        auto& f = *ptr;
        std::vector<double> pre = f.walls;
        if (pre.size() > 3) pre.resize(3);
        std::vector<double> last = f.walls;
        if (last.size() > 10) last = std::vector<double>(last.end() - 10, last.end());
        double premed = median(pre), lastmed = median(last);

        double dm=0.0, om=0.0; bool eq=true;
        if (theta_string(f.opt.theta) == theta_string(default_theta)) {
            std::vector<double> same;
            for (int i=0;i<11;++i) { auto z=ex.execute(f.exemplar, default_theta); same.push_back(z.wall_ms); eq = eq && z.ok; }
            dm = om = median(same);
        } else {
            auto paired = paired_wall(f, default_theta, f.opt.theta, 11);
            dm = std::get<0>(paired); om = std::get<1>(paired); eq = std::get<2>(paired);
        }
        double saving = dm - om;
        double speed = om > 0 ? dm / om : 0;
        bool meaningful = saving >= 0.050 && speed >= 1.02;
        if (meaningful) faster++;
        speed_sum += speed;
        double be = meaningful ? f.opt.exploration_regret / saving : -1;

        conv += f.opt.converged;
        total_candidates += f.opt.candidate_episodes;
        total_commits += f.opt.commits;
        book += f.opt.bookkeeping_ms;
        regret += f.opt.exploration_regret;

        std::string dsx;
        for (size_t k = 0; k < f.opt.dims.size(); ++k) {
            if (k) dsx += '|';
            dsx += dim_name(f.opt.dims[k]);
        }

        fsout << f.name << ',' << f.episodes << ',' << f.opt.converged << ',' << f.opt.convergence_episode << ','
              << esc(dsx) << ',' << f.opt.candidate_episodes << ',' << f.opt.commits << ',' << f.opt.bookkeeping_ms << ','
              << f.opt.exploration_regret << ',' << premed << ',' << lastmed << ','
              << (lastmed > 0 ? premed / lastmed : 0) << ',' << esc(theta_string(f.opt.theta)) << "\n";
        val << f.name << ',' << dm << ',' << om << ',' << speed << ',' << saving << ',' << f.opt.exploration_regret << ','
            << be << ',' << eq << ',' << esc(theta_string(f.opt.theta)) << "\n";
    }

    bool uniform = planner.vocab.audit();
    std::ofstream js(out / "runtime_summary.json");
    js << std::fixed << std::setprecision(6)
       << "{\n"
       << "  \"episodes\": " << eps.size() << ",\n"
       << "  \"success_rate\": " << double(okcount) / eps.size() << ",\n"
       << "  \"semantic_total\": " << total_sem << ",\n"
       << "  \"families\": " << families.size() << ",\n"
       << "  \"families_converged\": " << conv << ",\n"
       << "  \"families_certified_local\": " << certified_local << ",\n"
       << "  \"families_validation_faster\": " << faster << ",\n"
       << "  \"candidate_production_episodes\": " << total_candidates << ",\n"
       << "  \"extra_full_task_executions\": 0,\n"
       << "  \"optimizer_commits\": " << total_commits << ",\n"
       << "  \"optimizer_bookkeeping_ms\": " << book << ",\n"
       << "  \"exploration_regret_ms\": " << regret << ",\n"
       << "  \"baseline_anchor_rollbacks\": " << anchor_rollbacks << ",\n"
       << "  \"idle_refine_commits\": " << idle_refine_commits << ",\n"
       << "  \"idle_validation_wall_ms_test_only\": " << idle_test_wall_ms << ",\n"
       << "  \"mean_validation_speedup\": " << speed_sum / families.size() << ",\n"
       << "  \"peak_rss_kb\": " << peak << ",\n"
       << "  \"derived_final\": " << planner.vocab.derived.size() << ",\n"
       << "  \"uniform_vocabulary\": " << (uniform ? "true" : "false") << ",\n"
       << "  \"root_vocabulary\": [\"OBSERVE\", \"DERIVE\", \"COMMIT\"]\n"
       << "}\n";

    std::cout << "online_episodes=" << eps.size() << " success=" << okcount << " families=" << families.size()
              << " converged=" << conv << " certified=" << certified_local << " candidates=" << total_candidates
              << " commits=" << total_commits << " bookkeeping_ms=" << book << " regret_ms=" << regret
              << " anchor_rollbacks=" << anchor_rollbacks << " uniform=" << uniform << "\n";

    return (okcount == eps.size() && uniform && conv == families.size() && certified_local == families.size()) ? 0 : 4;
}

} // namespace runtime_descent

int main(int argc,char**argv){try{fs::path out="results_runtime_descent";bool online=false;for(int i=1;i<argc;i++){if(std::string(argv[i])=="--online-only")online=true;else if(i+1<argc&&std::string(argv[i])=="--out")out=argv[++i];}fs::create_directories(out);int base=run(out/"base_regression");if(base!=0)return base;if(online)return runtime_descent::run_optimizer_online(out/"runtime_descent_online");return runtime_descent::run_optimizer(out/"runtime_descent");}catch(const std::exception&e){std::cerr<<"fatal: "<<e.what()<<"\n";return 1;}}
