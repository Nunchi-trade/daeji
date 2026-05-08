import type { MutableRefObject } from 'react';
import { ValidatorNode } from './ValidatorNode';
import type { SceneState } from './sceneState';

interface Props {
  validatorCount: number;
  radius: number;
  sceneStateRef: MutableRefObject<SceneState>;
}

export function ValidatorRing({
  validatorCount,
  radius,
  sceneStateRef,
}: Props) {
  const nodes = Array.from({ length: validatorCount }, (_, i) => {
    const angle = (i / validatorCount) * Math.PI * 2;
    return (
      <ValidatorNode
        key={i}
        index={i}
        angle={angle}
        radius={radius}
        sceneStateRef={sceneStateRef}
      />
    );
  });

  return <group>{nodes}</group>;
}
