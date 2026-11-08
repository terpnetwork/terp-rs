// Drive the real edgar binary against discord-digital-twin and a fixture LCD.
// Serenity replaces the https://discord.com prefix, so EDGAR_DISCORD_API is
// the twin origin (http://127.0.0.1:<port>), not restUrl (which already ends
// in /api).
import { spawn } from 'node:child_process'
import { mkdirSync, readFileSync, writeFileSync, rmSync } from 'node:fs'
import { createServer, request as forward } from 'node:http'
import { tmpdir } from 'node:os'
import path from 'node:path'
import { ChannelType } from 'discord-api-types/v10'
import { DigitalDiscord } from 'discord-digital-twin'

const crate = path.resolve(new URL('.', import.meta.url).pathname, '../..')
const work = path.join(tmpdir(), `edgar-twin-${process.pid}`)
mkdirSync(path.join(work, 'data', 'docs'), { recursive: true })

let missed = 10
const lcd = createServer((req, res) => {
  const url = req.url || '/'
  const json = (body) => {
    res.writeHead(200, { 'content-type': 'application/json' })
    res.end(JSON.stringify(body))
  }
  if (url.includes('/cosmos/gov/v1/proposals/')) return json({ proposal: proposal() })
  if (url.includes('proposal_status=2')) return json({ proposals: [proposal()] })
  if (url.includes('/cosmos/gov/v1/proposals')) return json({ proposals: [] })
  if (url.includes('/ibc/core/channel/')) return json({ channels: [] })
  if (url.includes('/cosmos/staking/v1beta1/pool')) return json({ pool: { bonded_tokens: '1000000' } })
  if (url.includes('/cosmos/staking/v1beta1/validators')) {
    return json({
      validators: [
        {
          operator_address: 'terpvaloper1qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqq5q5lj',
          tokens: '1000000',
          jailed: false,
          status: 'BOND_STATUS_BONDED',
          description: { moniker: 'alpha' },
          consensus_pubkey: { key: 'AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=' },
        },
      ],
    })
  }
  if (url.includes('/cosmos/slashing/v1beta1/params')) {
    return json({
      params: { signed_blocks_window: '100', min_signed_per_window: '0.050000000000000000' },
    })
  }
  if (url.includes('/cosmos/slashing/v1beta1/signing_infos')) {
    return json({
      info: [
        {
          address: 'terpvalcons1hz7tqlmrgj6z4vzz2ryx5m5twhflmw7x7kp57q',
          missed_blocks_counter: missed,
          tombstoned: false,
        },
      ],
    })
  }
  if (url.includes('/cosmos/upgrade/')) return json({})
  if (url.includes('/blocks/latest')) return json({ block: { header: { height: '1000' } } })
  res.writeHead(404)
  res.end()
})

function proposal() {
  const end = new Date(Date.now() + 12 * 60 * 60 * 1000).toISOString()
  return {
    id: '70',
    status: 'PROPOSAL_STATUS_VOTING_PERIOD',
    title: 'upgrade v6.2',
    summary: 'fixture',
    voting_end_time: end,
    messages: [],
  }
}

const discord = new DigitalDiscord({
  // discord_http refuses a live token when EDGAR_DISCORD_API is set.
  botToken: 'dHdpbg==.twin.fake-bot-token',
  guild: { name: 'Edgar Twin' },
  channels: [
    { name: 'activity', type: ChannelType.GuildText },
    { name: 'gov-updates', type: ChannelType.GuildText },
  ],
  users: [{ username: 'Tester' }],
})

let bot
let logs = ''
let edge
try {
  await discord.start()
  // Serenity 0.12 calls GET /api/v10/gateway. This twin only implements
  // /gateway/bot, and a failed lookup makes serenity open gateway.discord.gg.
  edge = createServer((req, res) => {
    const url = req.url || '/'
    if (url === '/api/v10/gateway' || url.startsWith('/api/v10/gateway?')) {
      res.writeHead(200, { 'content-type': 'application/json' })
      res.end(JSON.stringify({ url: `ws://127.0.0.1:${discord.port}/gateway` }))
      return
    }
    const upstream = forward(
      {
        hostname: '127.0.0.1',
        port: discord.port,
        path: url,
        method: req.method,
        headers: { ...req.headers, host: `127.0.0.1:${discord.port}` },
      },
      (up) => {
        res.writeHead(up.statusCode || 502, up.headers)
        up.pipe(res)
      },
    )
    upstream.on('error', () => {
      if (!res.headersSent) res.writeHead(502)
      res.end()
    })
    req.pipe(upstream)
  })
  const edgePort = await new Promise((resolve) => edge.listen(0, '127.0.0.1', () => resolve(edge.address().port)))
  const lcdPort = await new Promise((resolve) => lcd.listen(0, '127.0.0.1', () => resolve(lcd.address().port)))
  const channels = await discord.prisma.channel.findMany()
  const activity = channels.find((c) => c.name === 'activity')
  const gov = channels.find((c) => c.name === 'gov-updates')
  if (!activity || !gov) throw new Error('twin channels missing')
  const origin = `http://127.0.0.1:${edgePort}`
  const built = spawn('cargo', ['+1.95.0', 'build', '--quiet', '--bin', 'discord-demo', '--manifest-path', path.join(crate, 'Cargo.toml')], {
    cwd: crate,
    stdio: ['ignore', 'pipe', 'pipe'],
  })
  built.stdout.on('data', (b) => { logs += b })
  built.stderr.on('data', (b) => { logs += b })
  const buildCode = await new Promise((resolve) => built.on('close', resolve))
  if (buildCode !== 0) throw new Error(`cargo build exited ${buildCode}`)

  bot = spawn(path.join(crate, 'target/debug/discord-demo'), [], {
    cwd: work,
    env: {
      ...process.env,
      EDGAR_DISCORD_API: origin,
      DISCORD_TOKEN: discord.botToken,
      DISCORD_GUILD_ID: discord.guildId,
      TERP_NODE_REST: `http://127.0.0.1:${lcdPort}`,
      CHAIN_ALERT_CHANNEL_ID: activity.id,
      ACTIVITY_CHANNEL_ID: activity.id,
      GOV_ALERT_CHANNEL_ID: gov.id,
      VALIDATOR_ALERT_CHANNEL_ID: activity.id,
      VALIDATOR_POLL_SECS: '1',
      CHAIN_POLL_SECS: '1',
      ASK_ENABLED: '1',
      ASK_CHANNEL_IDS: activity.id,
      ASK_ROLE_IDS: discord.guildId,
      ASK_COOLDOWN_SECS: '0',
      ASK_MAX_CONCURRENT: '2',
      LLM_BASE_URL: 'http://127.0.0.1:9/v1',
      LLM_API_KEY: 'twin',
      LLM_KIND: 'openai',
      RUST_LOG: 'discord_demo=info',
    },
    stdio: ['ignore', 'pipe', 'pipe'],
  })
  bot.stdout.on('data', (b) => { logs += b })
  bot.stderr.on('data', (b) => { logs += b })

  const userId = await discord.getFirstNonBotUserId()
  await waitFor(async () => {
    const n = await discord.prisma.applicationCommand.count({ where: { name: 'edgar' } })
    return n > 0
  }, 90_000, 'edgar command registration')

  const actor = discord.channel(activity.id).user(userId)
  await actor.runSlashCommand({
    name: 'edgar',
    options: [
      { name: 'ask', type: 1, options: [
        { name: 'topic', type: 3, value: 'terp-docs' },
        { name: 'question', type: 3, value: 'what is a validator' },
      ] },
    ],
  })
  const thread = await discord.channel(activity.id).waitForThread({ timeout: 20_000 })
  const trace = await discord.channel(thread.id).waitForMessage({
    timeout: 20_000,
    predicate: (m) => (m.content || '').includes('what is a validator'),
  })
  if (!trace.content.includes('what is a validator')) throw new Error(`trace missing question: ${trace.content}`)

  writeFileSync(path.join(work, 'data', 'alert-threads.json'), JSON.stringify({ [thread.id]: 'proposal 70 title' }))
  await discord.channel(thread.id).user(userId).runSlashCommand({
    name: 'edgar',
    options: [
      { name: 'ask', type: 1, options: [
        { name: 'topic', type: 3, value: 'terp-docs' },
        { name: 'question', type: 3, value: 'second ask' },
      ] },
    ],
  })
  const prefixed = await discord.channel(thread.id).waitForMessage({
    timeout: 20_000,
    predicate: (m) => (m.content || '').includes('proposal 70'),
  })
  if (!prefixed.content.includes('proposal 70')) throw new Error('thread prefix missing')

  await waitFor(async () => {
    const msgs = await discord.prisma.message.findMany({ where: { channelId: gov.id } })
    return msgs.some((m) => (m.content || '').includes('proposal #70') || (m.content || '').includes('#70'))
  }, 20_000, 'gov reminder')

  await waitFor(async () => {
    try {
      return readFileSync(path.join(work, 'data', 'closest-jail.txt'), 'utf8').includes('alpha')
    } catch {
      return false
    }
  }, 30_000, 'validator seed line')
  missed = 96
  await waitFor(async () => {
    const msgs = await discord.prisma.message.findMany({ where: { channelId: activity.id } })
    return msgs.some((m) => (m.content || '').includes('jail line') || (m.content || '').includes('96/'))
  }, 20_000, 'jail line')

  console.log('discord-twin e2e ok')
} catch (err) {
  console.error(err)
  const msgs = await discord.prisma.message.findMany().catch(() => [])
  for (const m of msgs) console.error(`msg ${m.channelId}: ${(m.content || '').slice(0, 240)}`)
  if (bot && logs) console.error(logs.slice(-4000))
  process.exitCode = 1
} finally {
  if (bot) bot.kill('SIGTERM')
  lcd.close()
  if (edge) edge.close()
  await discord.stop().catch(() => {})
  rmSync(work, { recursive: true, force: true })
}

async function waitFor(fn, timeout, label) {
  const start = Date.now()
  while (Date.now() - start < timeout) {
    if (await fn()) return
    await new Promise((r) => setTimeout(r, 250))
  }
  throw new Error(`timed out: ${label}`)
}
