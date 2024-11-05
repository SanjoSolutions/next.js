import { retry } from 'next-test-utils'
import { nextTestSetup } from 'e2e-utils'

describe('app-dir - fetch warnings', () => {
  const { next, skipped, isNextDev } = nextTestSetup({
    skipDeployment: true,
    files: __dirname,
  })

  if (skipped) {
    return
  }

  beforeAll(async () => {
    // we don't need verbose logging (enabled by default in this Next app) for these tests to work
    // we avoid enabling it since it's not currently compatible with Turbopack.
    await next.stop()
    await next.deleteFile('next.config.js')
    await next.start()
  })

  if (isNextDev) {
    describe('static pages', () => {
      beforeAll(async () => {
        await next.fetch('/cache-revalidate')
      })
      describe('force-cache and revalidate: 0', () => {
        it('should log when request input is a string', async () => {
          await retry(() => {
            expect(next.cliOutput).toInclude(`
 │ GET https://next-data-api-endpoint.vercel.app/api/random?request-string
 │ │ ⚠ Specified "cache: force-cache" and "revalidate: 0", only one should be specified.`)
          })
        })

        it('should log when request input is a Request instance', async () => {
          await retry(() => {
            expect(next.cliOutput).toInclude(`
 │ GET https://next-data-api-endpoint.vercel.app/api/random?request-input-cache-override
 │ │ ⚠ Specified "cache: force-cache" and "revalidate: 0", only one should be specified.`)
          })
        })

        it('should not log when not overriding cache within the Request object', async () => {
          await retry(() => {
            expect(next.cliOutput).not.toInclude(`
 │ GET https://next-data-api-endpoint.vercel.app/api/random?request-input
 │ │ ⚠ Specified "cache:`)
          })
        })
      })

      describe('no-store and revalidate > 0', () => {
        it('should log when request input is a string', async () => {
          await retry(() => {
            expect(next.cliOutput).toInclude(`
 │ GET https://next-data-api-endpoint.vercel.app/api/random?no-store-request-string
 │ │ ⚠ Specified "cache: no-store" and "revalidate: 3", only one should be specified.`)
          })
        })

        it('should log when request input is a Request instance', async () => {
          await retry(() => {
            expect(next.cliOutput).toInclude(`
 │ GET https://next-data-api-endpoint.vercel.app/api/random?no-store-request-input-cache-override
 │ │ ⚠ Specified "cache: no-store" and "revalidate: 3", only one should be specified.`)
          })
        })
      })
    })

    describe('dynamic pages', () => {
      beforeAll(async () => {
        await next.fetch('/cache-revalidate/force-dynamic')
      })
      it('should log a warning when specifying a revalidate value without an explicit cache config', async () => {
        await retry(() => {
          expect(next.cliOutput).toInclude(`
 │ GET https://next-data-api-endpoint.vercel.app/api/random?revalidate-3
 │ │ ⚠ Specified "dynamic: 'force-dynamic'" and "revalidate: 3" without explicitly caching the fetch. This fetch will be treated as an uncached fetch.`)
        })
      })
    })
  } else {
    it('should not log fetch warnings in production', async () => {
      await retry(() => {
        expect(next.cliOutput).not.toInclude(
          '⚠ Specified "cache: force-cache" and "revalidate: 3", only one should be specified.'
        )
      })
    })
  }
})
