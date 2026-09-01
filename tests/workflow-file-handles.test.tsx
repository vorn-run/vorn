// @vitest-environment jsdom
import { describe, it, expect, vi, afterEach } from 'vitest'
import {
  pickWorkflowFile,
  readDroppedWorkflowFile
} from '../src/renderer/lib/workflow-files'

const CONTENTS = '{"version":1,"slug":"x","name":"X","nodes":[],"edges":[]}'

function jsonFile(name = 'x.vorn-workflow.json'): File {
  return new File([CONTENTS], name, { type: 'application/json' })
}

/** Stand in for the native picker: the click is what opens it. */
function whenPickerOpens(handler: (input: HTMLInputElement) => void) {
  return vi.spyOn(HTMLInputElement.prototype, 'click').mockImplementation(function (
    this: HTMLInputElement
  ) {
    handler(this)
  })
}

afterEach(() => {
  vi.restoreAllMocks()
})

describe('reading a dropped file', () => {
  it('reads the workflow file out of the drop', async () => {
    const picked = await readDroppedWorkflowFile([jsonFile()] as unknown as FileList)
    expect(picked).toEqual({ name: 'x.vorn-workflow.json', contents: CONTENTS })
  })

  it('ignores what came alongside it', async () => {
    const files = [
      new File(['n'], 'notes.txt', { type: 'text/plain' }),
      jsonFile('second.json')
    ] as unknown as FileList
    expect((await readDroppedWorkflowFile(files))?.name).toBe('second.json')
  })

  it('answers nothing for a drop with no workflow file in it', async () => {
    const files = [new File(['n'], 'notes.txt')] as unknown as FileList
    expect(await readDroppedWorkflowFile(files)).toBeNull()
    expect(await readDroppedWorkflowFile(null)).toBeNull()
  })
})

describe('asking for a file', () => {
  it('hands back the text of the file that was chosen', async () => {
    whenPickerOpens((input) => {
      Object.defineProperty(input, 'files', { value: [jsonFile()] })
      input.dispatchEvent(new Event('change'))
    })

    expect(await pickWorkflowFile()).toEqual({
      name: 'x.vorn-workflow.json',
      contents: CONTENTS
    })
  })

  it('answers nothing when the picker is dismissed', async () => {
    whenPickerOpens((input) => input.dispatchEvent(new Event('cancel')))
    expect(await pickWorkflowFile()).toBeNull()
  })

  it('answers nothing when the picker closes with no file', async () => {
    whenPickerOpens((input) => {
      Object.defineProperty(input, 'files', { value: [] })
      input.dispatchEvent(new Event('change'))
    })
    expect(await pickWorkflowFile()).toBeNull()
  })

  it('leaves no input behind in the document', async () => {
    whenPickerOpens((input) => {
      Object.defineProperty(input, 'files', { value: [jsonFile()] })
      input.dispatchEvent(new Event('change'))
    })

    await pickWorkflowFile()
    expect(document.querySelectorAll('input[type="file"]')).toHaveLength(0)
  })
})
