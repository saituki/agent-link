// マイクの音声を Live API 用の PCM(16bit・16000Hz・モノラル)に変換する部品．
//
// なぜ必要か:
// Live API は「生の PCM」という形式の音声しか受け付けない．
// ブラウザの MediaRecorder は webm などの圧縮形式しか出せないため，
// ここで1つずつ数字に変換して送る．
//
// 仕組み(初心者向け):
// マイクの音は「-1.0〜1.0 の小数」の列で届く．
// それを「-32768〜32767 の整数」(Int16)に変換する．
// AudioContext のサンプリングレートが 16000Hz でない場合(iPhone でよくある)は，
// 線形補間(間の値をまっすぐつなぐ)で 16000Hz に揃える．
// 100ms 分(1600個)たまったらメインスレッドに渡す．

class PcmCapture extends AudioWorkletProcessor {
  constructor() {
    super();
    // 送りたいサンプリングレート(Hz)．メインスレッドから教えてもらう．
    this.targetRate = 16000;
    // 変換前の余りをためておく場所(リサンプリング用)．
    this.carry = [];
    // 100ms 分 = 1600 サンプル．
    this.chunkSize = 1600;
    this.buffer = [];
    this.port.onmessage = (e) => {
      if (e.data && e.data.targetRate) this.targetRate = e.data.targetRate;
      if (e.data && e.data.chunkSize) this.chunkSize = e.data.chunkSize;
    };
  }

  process(inputs) {
    const input = inputs[0];
    if (!input || input.length === 0) return true;
    const channel = input[0];
    if (!channel || channel.length === 0) return true;

    // 前回の余りと今回の入力をつなげる．
    const joined = this.carry.length > 0
      ? [...this.carry, ...channel]
      : [...channel];
    this.carry = [];

    const sourceRate = sampleRate;
    if (sourceRate === this.targetRate) {
      // レートが同じならそのまま整数に変換する．
      for (const v of joined) this.buffer.push(floatToInt16(v));
    } else {
      // 違う場合は線形補間で揃える．
      // 例: 48000Hz → 16000Hz なら 3個に1個の割合で拾う．
      const ratio = sourceRate / this.targetRate;
      const outLen = Math.floor(joined.length / ratio);
      for (let i = 0; i < outLen; i++) {
        const pos = i * ratio;
        const idx = Math.floor(pos);
        const frac = pos - idx;
        const a = joined[idx];
        const b = idx + 1 < joined.length ? joined[idx + 1] : a;
        this.buffer.push(floatToInt16(a + (b - a) * frac));
      }
      // 端数(次回に持ち越す分)を覚えておく．
      const consumed = Math.floor(outLen * ratio);
      this.carry = joined.slice(consumed);
    }

    // 100ms 分たまったらメインスレッドに渡す．
    // 必ず Int16Array(1サンプル2バイトの整数)で渡す．
    // (普通の配列や Float32Array で渡すと，送り側で形式を取り違えて
    // 雑音として届いてしまうため．Live API は Int16 のバイト列を要求する)
    while (this.buffer.length >= this.chunkSize) {
      const chunk = new Int16Array(this.buffer.slice(0, this.chunkSize));
      this.buffer = this.buffer.slice(this.chunkSize);
      this.port.postMessage({ pcm16: chunk });
    }
    return true;
  }
}

// -1.0〜1.0 の小数を -32768〜32767 の整数に変換する．
function floatToInt16(v) {
  const clamped = Math.max(-1, Math.min(1, v));
  return Math.round(clamped * 0x7fff);
}

registerProcessor("pcm-capture", PcmCapture);
