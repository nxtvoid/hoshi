import { describe, expect, test } from 'bun:test'
import { moveProject, orderProjects } from '../../src/state/project-order'

const projects = [{ id: 'a' }, { id: 'b' }, { id: 'c' }, { id: 'd' }]
const ids = (list: { id: string }[]) => list.map((project) => project.id)

describe('orderProjects', () => {
  test('keeps the given order without a saved one', () => {
    expect(ids(orderProjects(projects, []))).toEqual(['a', 'b', 'c', 'd'])
  })

  test('follows the saved order and puts new projects last', () => {
    expect(ids(orderProjects(projects, ['c', 'a']))).toEqual([
      'c',
      'a',
      'b',
      'd'
    ])
  })

  test('ignores ids of removed projects', () => {
    expect(ids(orderProjects(projects, ['x', 'd', 'b']))).toEqual([
      'd',
      'b',
      'a',
      'c'
    ])
  })
})

describe('moveProject', () => {
  test('moves down', () => {
    expect(moveProject(['a', 'b', 'c', 'd'], 'a', 'c')).toEqual([
      'b',
      'c',
      'a',
      'd'
    ])
  })

  test('moves up', () => {
    expect(moveProject(['a', 'b', 'c', 'd'], 'd', 'b')).toEqual([
      'a',
      'd',
      'b',
      'c'
    ])
  })

  test('leaves the order alone for unknown ids', () => {
    expect(moveProject(['a', 'b'], 'a', 'x')).toEqual(['a', 'b'])
  })
})
