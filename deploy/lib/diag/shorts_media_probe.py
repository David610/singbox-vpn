#!/usr/bin/env python3
"""Native-ish YouTube Shorts media probe (diagnostic only; NOT device acceptance).
Per Short: sample <video>.currentTime every 0.5 s for DWELL s; count videoplayback bytes; flag the
155-byte 'yt-ump' refusal; keep bot-challenge and 'unavailable' separate. JSON to stdout last line."""
import argparse,json,sys,time,re
from playwright.sync_api import sync_playwright
ap=argparse.ArgumentParser();ap.add_argument('--proxy');ap.add_argument('--quic',action='store_true');ap.add_argument('--channel',default=None)
ap.add_argument('--ids',required=True);ap.add_argument('--dwell',type=float,default=25);ap.add_argument('--label',default='run')
a=ap.parse_args();ids=a.ids.split(',')
CONSENT={"name":"SOCS","value":"CAESEwgDEgk0ODE3Nzk3MjQaAmVuIAEaBgiA_LyaBg","domain":".youtube.com","path":"/"}
JS="""()=>{const v=[...document.querySelectorAll('video')].find(x=>x.readyState>0||x.currentTime>0)||document.querySelector('video');
const t=(document.body.innerText||'').toLowerCase();
return {path:location.pathname,time:v?+v.currentTime.toFixed(2):null,ready:v?v.readyState:null,paused:v?v.paused:null,
buf:v&&v.buffered.length?+v.buffered.end(v.buffered.length-1).toFixed(1):0,
bot:t.includes('not a bot')||t.includes('confirm you')&&t.includes('sign in'),unavail:t.includes('video unavailable')||t.includes('this video isn')}}"""
res=[]
with sync_playwright() as p:
    args=['--mute-audio','--autoplay-policy=no-user-gesture-required']+([] if a.quic else ['--disable-quic'])
    b=p.chromium.launch(channel=a.channel,headless=True,proxy={"server":a.proxy} if a.proxy else None,args=args)  # sandbox left ON
    ctx=b.new_context(**p.devices['iPhone 13'],locale='en-US',timezone_id='Europe/Berlin');ctx.add_cookies([CONSENT])
    for vid in ids:
        pg=ctx.new_page();media=[];fin=[]
        def onfin(req):
            if 'googlevideo.com/videoplayback' in req.url:
                try:
                    s=req.sizes();r=req.response();fin.append((r.status if r else None,s.get('responseBodySize',0)))
                except Exception:pass
        pg.on('requestfinished',onfin)
        t0=time.time();err=None
        try:pg.goto(('https://www.youtube.com/watch?v='+vid[6:]) if vid.startswith('watch:') else ('https://www.youtube.com/shorts/'+vid),wait_until='domcontentloaded',timeout=30000)
        except Exception as e:err=type(e).__name__
        samples=[];start=None
        while time.time()-t0<a.dwell+5:
            try:s=pg.evaluate(JS)
            except Exception:s=None
            if s:
                samples.append((round(time.time()-t0,1),s['time'],s['paused'],s['bot'],s['unavail'],s['path'],s['buf']))
                if start is None and s['time'] and s['time']>0.8:start=round(time.time()-t0,1)
            time.sleep(0.5)
        times=[x[1] for x in samples if x[1] is not None]
        # max stall: longest span where currentTime did not advance (>0.05) after playback started
        stall=0;last_t=None;last_ts=None
        for ts,tm,paused,_,_,_,_ in samples:
            if tm is None:continue
            if last_t is None or abs(tm-last_t)>0.05:last_t,last_ts=tm,ts  # loop/seek counts as progress
            else:stall=max(stall,ts-last_ts)
        small=sum(1 for st,sz in fin if sz is not None and sz<=200 and st in(200,206))
        ok_bytes=sum(sz for st,sz in fin if st in(200,206) and sz and sz>200)
        r=dict(id=vid,err=err,started=start is not None,start_s=start,played_s=round((max(times)-min(times)),1) if times else 0,
               max_stall_s=round(stall,1),bot_challenge=any(x[3] for x in samples),unavailable=any(x[4] for x in samples),
               still_on_shorts=all(x[5].startswith('/shorts/') for x in samples) if samples else False,
               videoplayback_reqs=len(fin),refusal_155B_like=small,media_bytes=ok_bytes,statuses=sorted({st for st,_ in fin if st}))
        r['verdict']=('BOT_CHALLENGE' if r['bot_challenge'] else 'PLAYED' if (r['started'] and r['played_s']>=a.dwell*0.6 and r['max_stall_s']<4) else 'FROZE' if r['started'] else 'REFUSED' if (r['unavailable'] or small) else 'NO_START')
        res.append(r);pg.close()
    b.close()
for r in res:print('%-12s %-13s start=%-5s played=%5.1fs maxstall=%4.1fs vp_reqs=%3d refusal-like=%d media=%6dKB st=%s'%(r['id'],r['verdict'],r['start_s'],r['played_s'],r['max_stall_s'],r['videoplayback_reqs'],r['refusal_155B_like'],r['media_bytes']//1000,r['statuses']))
print('SUMMARY',a.label,json.dumps({k:sum(1 for r in res if r['verdict']==k) for k in ('PLAYED','FROZE','REFUSED','BOT_CHALLENGE','NO_START')}))
json.dump(res,open('result_%s.json'%a.label,'w'))
