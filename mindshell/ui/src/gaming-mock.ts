/** Explicit browser fixtures; never loaded by the native host. */
export function gamingMock(): (p: Record<string, unknown>) => unknown {
  const meta: Record<string, Record<string, unknown>> = {};
  let cfg: Record<string, unknown> = {};
  const now = Date.now()/1000;
  const sessions = [{id:'preview-session',game:'steam:1091500',started:now-3600,active:true,suspended:true}];
  const events: unknown[] = [];
  const saves: { id: string; time: number; files: number; cloud: boolean }[] = [];
  const archive: Record<string, unknown> = {};
  return p => {
    const game = String(p.game || '');
    switch(p.action) {
      case 'config.get': return {...cfg,steam_connected:!!cfg.steam_id};
      case 'config.set': cfg = {...cfg,...p.settings as object}; delete cfg.steam_key; delete cfg.obs_password; return cfg;
      case 'config.disconnect': delete cfg.steam_id; return cfg;
      case 'sessions': return sessions.map(s=>({...s}));
      case 'session.setup': return {command:`mindos-play run '${game}' -- %command%`,instructions:'Steam: Properties → General → Launch options. Browser preview uses example sessions.'};
      case 'session.suspend': case 'session.resume': { const s=sessions.find(s=>s.game===game); if(!s) throw new Error('This game has no managed session.'); s.suspended=p.action==='session.suspend'; events.push({time:now,action:p.action,game}); return {...s,transition_ms:24}; }
      case 'metadata.get': return meta[game] || {};
      case 'metadata.set': meta[game]={...meta[game],...p.settings as object};return meta[game];
      case 'library.state': return { metadata:meta, storage:archive, sessions };
      case 'saves.list': return saves;
      case 'saves.backup': if(!meta[game]?.save_path) throw new Error('Choose a save folder first.'); saves.push({id:String(Date.now()),time:now,files:6,cloud:!!p.cloud});return saves.at(-1);
      case 'saves.restore': return {restored:p.revision};
      case 'storage.list': return archive;
      case 'storage.plan': if(!cfg.cold_folder) throw new Error('Choose a cold storage folder in Connections.'); return {source:'/games/example',destination:'/cold/example',bytes:42*1024**3,free:420*1024**3};
      case 'storage.move': if(p.restore) delete archive[game];else archive[game]={cold:'/cold/example'};return {};
      case 'downloads': return {items:[{id:'steam:570',name:'Dota 2',percent:63,downloaded:6.3*1024**3,total:10*1024**3}],note:'Browser preview · example download'};
      case 'friends': if(!cfg.steam_id) throw new Error('Connect Steam in Gaming connections to see friends.');return {friends:[{steamid:'76561198000000001',personaname:'Preview player',personastate:1,gameextrainfo:'Dota 2',gameid:'570'}]};
      case 'achievements': return {total:20,unlocked:12};
      case 'history': return [{...sessions[0],ended:now,active:false,stats:{avg_fps:142,p99_ms:12.4,stutters:3,points:[7,7.2,6.9,12.4,7.3,8.1,7.2]}}];
      case 'analyze': return {summary:'Browser preview · example trace: 142 FPS average, 12.4 ms p99.',suggestions:['Compare another recording after changing one setting.']};
      case 'audio.streams':return [{id:1,name:'Example game',volume:80},{id:2,name:'Browser',volume:100}];
      case 'audio.duck':case 'audio.restore':case 'audio.volume':return {};
      case 'activity':return events;
      case 'boot':return {summary:'Browser preview · boot measurements are available on MindOS.',services:[]};
      default:throw new Error(`Unsupported preview gaming action: ${p.action}`);
    }
  };
}
