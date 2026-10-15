// Decode one visible video's first frame at a time, then release its decoder.
// Cards retain only a small JPEG; full players are mounted on demand.
let pending: Promise<void> = Promise.resolve();

export function firstVideoFrame(source: string, signal: AbortSignal): Promise<string> {
  const result = pending.then(() => readFrame(source, signal));
  pending = result.then(() => {}, () => {});
  return result;
}

function readFrame(source: string, signal: AbortSignal): Promise<string> {
  if (signal.aborted) return Promise.reject(new DOMException('Aborted', 'AbortError'));
  return new Promise((resolve, reject) => {
    const video = document.createElement('video');
    let finished = false;
    const timer = window.setTimeout(() => finish(undefined, new Error('视频首帧加载超时')), 8000);
    function finish(poster?: string, error?: Error) {
      if (finished) return;
      finished = true;
      window.clearTimeout(timer);
      signal.removeEventListener('abort', abort);
      video.removeEventListener('loadeddata', capture);
      video.removeEventListener('error', failed);
      video.pause();
      video.removeAttribute('src');
      video.load();
      if (poster) resolve(poster); else reject(error ?? new Error('视频首帧不可用'));
    }
    function abort() { finish(undefined, new DOMException('Aborted', 'AbortError')); }
    function failed() { finish(undefined, new Error('当前播放器无法读取视频首帧')); }
    function capture() {
      try {
        if (!video.videoWidth || !video.videoHeight) throw new Error('视频没有可读取的画面');
        const ratio = Math.min(1, 960 / video.videoWidth, 540 / video.videoHeight);
        const canvas = document.createElement('canvas');
        canvas.width = Math.max(1, Math.round(video.videoWidth * ratio));
        canvas.height = Math.max(1, Math.round(video.videoHeight * ratio));
        const context = canvas.getContext('2d');
        if (!context) throw new Error('视频首帧不可用');
        context.drawImage(video, 0, 0, canvas.width, canvas.height);
        const poster = canvas.toDataURL('image/jpeg', 0.75);
        if (!poster.startsWith('data:image/') || poster.length > 768 * 1024) throw new Error('视频首帧不可用');
        finish(poster);
      } catch (error) { finish(undefined, error instanceof Error ? error : new Error(String(error))); }
    }
    video.muted = true;
    video.playsInline = true;
    video.preload = 'auto';
    video.crossOrigin = 'anonymous';
    video.addEventListener('loadeddata', capture);
    video.addEventListener('error', failed);
    signal.addEventListener('abort', abort, { once: true });
    video.src = source;
    video.load();
  });
}
