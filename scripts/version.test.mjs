// @vitest-environment node
// Číslo verze žije na čtyřech místech a CHANGELOG.md je jeho evidence.
// Tenhle test hlídá, aby se nerozjely a aby držely SemVer 2.0.0-rc.2.

import { readFileSync } from 'node:fs'
import { describe, expect, it } from 'vitest'

import { compareVersions, extractRelease, isValidVersion, listVersions, releaseDate } from './changelog.mjs'

const read = (path) => readFileSync(new URL(`../${path}`, import.meta.url), 'utf8')

const packageVersion = JSON.parse(read('package.json')).version
const tauriVersion = JSON.parse(read('src-tauri/tauri.conf.json')).version
const cargoVersion = /^\[package\][\s\S]*?^version = "([^"]+)"/m.exec(read('src-tauri/Cargo.toml'))?.[1]
const lockVersion = /name = "skylimit"\r?\nversion = "([^"]+)"/.exec(read('Cargo.lock'))?.[1]
const changelog = read('CHANGELOG.md')

describe('číslo verze', () => {
  it('je všude stejné: package.json, tauri.conf.json, Cargo.toml a Cargo.lock', () => {
    expect(isValidVersion(packageVersion), packageVersion).toBe(true)
    expect(tauriVersion).toBe(packageVersion)
    expect(cargoVersion).toBe(packageVersion)
    expect(lockVersion).toBe(packageVersion)
  })

  it('má v CHANGELOG.md vlastní sekci s datem, ze které vznikne text vydání', () => {
    expect(extractRelease(changelog, packageVersion)).not.toBeNull()
    expect(releaseDate(changelog, packageVersion)).toMatch(/^\d{4}-\d{2}-\d{2}$/)
  })
})

describe('CHANGELOG.md', () => {
  const versions = listVersions(changelog)

  it('odkazuje na SemVer 2.0.0-rc.2 a má sekci [Nevydáno]', () => {
    expect(changelog).toContain('https://semver.org/spec/v2.0.0-rc.2.html')
    expect(changelog).toMatch(/^## \[Nevydáno\]$/m)
  })

  it('každá verze je platná, má datum a text a objeví se jen jednou', () => {
    expect(versions.length).toBeGreaterThan(0)
    expect(new Set(versions).size).toBe(versions.length)
    for (const v of versions) {
      expect(isValidVersion(v), v).toBe(true)
      expect(releaseDate(changelog, v), v).toMatch(/^\d{4}-\d{2}-\d{2}$/)
      expect(extractRelease(changelog, v), v).not.toBeNull()
    }
  })

  it('nejnovější verze je nahoře (přednost podle pravidla 11)', () => {
    const sorted = [...versions].sort((a, b) => compareVersions(b, a))
    expect(versions).toEqual(sorted)
    expect(versions[0]).toBe(packageVersion)
  })

  it('data vydání nejdou proti pořadí verzí', () => {
    const dates = versions.map((v) => releaseDate(changelog, v))
    expect([...dates].sort().reverse()).toEqual(dates)
  })

  it('používá jen kategorie Keep a Changelog', () => {
    const allowed = ['Přidáno', 'Změněno', 'Zastaralé', 'Odstraněno', 'Opraveno', 'Zabezpečení']
    const used = [...changelog.matchAll(/^### (.+)$/gm)].map((m) => m[1].trim())
    for (const heading of used) expect(allowed, heading).toContain(heading)
  })
})
