const ADJECTIVES = [
  'gilded',
  'marble',
  'ornate',
  'sacred',
  'divine',
  'golden',
  'silver',
  'crimson',
  'ivory',
  'velvet',
  'noble',
  'royal',
  'regal',
  'ancient',
  'baroque',
  'classical',
  'tuscan',
  'florentine',
  'venetian',
  'emerald',
  'amber',
  'obsidian',
  'bronze',
  'sienna',
  'scarlet'
]

const NOUNS = [
  'fresco',
  'madrigal',
  'etching',
  'sketch',
  'triptych',
  'inkwell',
  'study',
  'canvas',
  'palette',
  'tableau',
  'vellum',
  'relic',
  'mosaic',
  'statue',
  'chapel',
  'garden',
  'fountain',
  'frieze',
  'archive',
  'folio',
  'portrait',
  'scroll',
  'chronicle',
  'stanza',
  'muse'
]

function pick<T>(arr: T[]): T {
  return arr[Math.floor(Math.random() * arr.length)]
}

export function generateWorktreeName(): string {
  return `${pick(ADJECTIVES)}-${pick(NOUNS)}`
}
