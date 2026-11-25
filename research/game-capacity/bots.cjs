'use strict';
// Isolated localhost only. No account credentials or production auth changes.
const mineflayer = require('mineflayer')
const count = Number(process.argv[2])
const port = Number(process.argv[3])
if (!Number.isInteger(count) || count < 1 || count > 16 || !Number.isInteger(port) || port < 1024 || port > 65535) throw Error('invalid lab parameters')
const bots = []
let stopped = false
let ready = 0
let trafficStart = null
let lastTraffic = null
let timer = null
let peakRx = 0
let peakTx = 0
function emit(event, value) { console.log(JSON.stringify({event, ...value})) }
function trafficSnapshot() {
  return {
    at: performance.now(),
    rx: bots.reduce((sum, {socket}) => sum + socket.bytesRead, 0),
    tx: bots.reduce((sum, {socket}) => sum + socket.bytesWritten, 0)
  }
}
function sampleTraffic() {
  const current = trafficSnapshot()
  const seconds = (current.at - lastTraffic.at) / 1000
  if (seconds >= 0.9) {
    peakRx = Math.max(peakRx, (current.rx - lastTraffic.rx) / seconds)
    peakTx = Math.max(peakTx, (current.tx - lastTraffic.tx) / seconds)
  }
  lastTraffic = current
}
function stop() {
  if (stopped) return
  stopped = true
  if (timer) clearInterval(timer)
  const stats = bots.map(({bot, start, socket}) => ({
    name: bot.username, spawned: Boolean(start),
    displacement: start && bot.entity ? bot.entity.position.distanceTo(start) : null,
    received: socket?.bytesRead ?? null, sent: socket?.bytesWritten ?? null,
    ping: bot.player?.ping ?? null
  }))
  const finalTraffic = trafficStart && trafficSnapshot()
  const traffic = finalTraffic && {
    seconds: (finalTraffic.at - trafficStart.at) / 1000,
    received_after_all_spawned: finalTraffic.rx - trafficStart.rx,
    sent_after_all_spawned: finalTraffic.tx - trafficStart.tx,
    peak_sampled_rx_bytes_per_second: Math.round(peakRx),
    peak_sampled_tx_bytes_per_second: Math.round(peakTx)
  }
  emit('summary', {stats, traffic})
  for (const {bot} of bots) bot.quit()
  setTimeout(() => process.exit(0), 500).unref()
}
function fail(message) {
  if (stopped) return
  emit('failure', {message: String(message)})
  process.exit(1)
}
process.stdin.resume()
process.stdin.on('data', stop)
process.on('SIGTERM', stop)
for (let i = 0; i < count; i++) {
  setTimeout(() => {
    if (stopped) return
    const bot = mineflayer.createBot({host:'127.0.0.1', port, username:'CapacityBot'+i, auth:'offline', version:'26.1', viewDistance:'short', hideErrors:true})
    const record = {bot, start:null, socket:null}
    bots.push(record)
    bot.on('error', fail)
    bot.on('kicked', fail)
    bot.on('end', reason => fail('disconnected: '+reason))
    bot.once('spawn', () => {
      record.start = bot.entity.position.clone()
      record.socket = bot._client.socket
      bot.setControlState('forward', true)
      bot.setControlState('jump', true)
      bot.look(i * Math.PI * 2 / count, 0, true).catch(fail)
      if (++ready === count) {
        if (bots.some(({socket}) => !socket)) return fail('socket unavailable')
        trafficStart = lastTraffic = trafficSnapshot()
        timer = setInterval(sampleTraffic, 1000)
        emit('ready', {count})
      }
    })
  }, i * 400)
}
