import { useEffect, useState } from 'react';
import { desktopCancelDownload } from './runtime';
import { friendlyError } from './errors';
interface Progress {id:string;filename:string;written?:number;total?:number|null;finished:boolean;cancelled?:boolean;error?:string}
export function Downloads() {
  const [items,setItems] = useState<Progress[]>([]);
  useEffect(() => {
    const update = (e: Event) => {const data=(e as CustomEvent<Progress>).detail;setItems(items=>[...items.filter(item=>item.id!==data.id), {...items.find(item=>item.id===data.id),...data}].slice(-4));};
    window.addEventListener('desktop-download',update);return()=>window.removeEventListener('desktop-download',update);
  },[]);
  if(!items.length) return null;
  return <aside className="desktop-downloads" aria-live="polite">{items.map(item=><div key={item.id}><strong>{item.filename}</strong><p>{item.error?friendlyError(item.error):item.finished?(item.cancelled?'已取消':'已保存到电脑'):`已下载 ${((item.written??0)/1048576).toFixed(1)} MB${item.total?` / ${(item.total/1048576).toFixed(1)} MB`:''}`}</p>{!item.finished?<button onClick={()=>void desktopCancelDownload(item.id)}>取消下载</button>:<button onClick={()=>setItems(items=>items.filter(i=>i.id!==item.id))}>关闭</button>}</div>)}</aside>;
}
