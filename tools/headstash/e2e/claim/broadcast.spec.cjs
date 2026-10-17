// Headless click of Broadcast ProcessHeadstash. HTTP 200 on the HTML file is not this test.
const { chromium } = require('playwright')

async function main() {
  const url = process.env.HEADSTASH_CLAIM_URL || 'http://127.0.0.1:19820/'
  const browser = await chromium.launch({ headless: true })
  const page = await browser.newPage()
  await page.goto(url, { waitUntil: 'domcontentloaded' })
  await page.getByRole('button', { name: 'Broadcast ProcessHeadstash' }).click()
  await page.waitForFunction(() => {
    const text = document.getElementById('out')?.textContent || ''
    return text.includes('"txhash"') || text.includes('no txhash') || text.includes('Refusing') || text.includes('Failed to fetch') || text.includes('broadcast.json')
  }, null, { timeout: 180000 })
  const text = await page.locator('#out').innerText()
  await page.getByRole('button', { name: 'Query epoch root' }).click()
  await browser.close()
  const hashed = /"txhash"\s*:\s*"([0-9A-Fa-f]{64})"/.exec(text)
  if (!hashed) {
    console.error(text.slice(0, 500))
    process.exit(1)
  }
  console.log(`PLAYWRIGHT_CLICK_OK txhash=${hashed[1]}`)
}

main().catch((err) => {
  console.error(err && err.stack ? err.stack : String(err))
  process.exit(1)
})
