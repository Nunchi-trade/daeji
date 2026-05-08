import { jsx as _jsx } from "react/jsx-runtime";
import { ValidatorNode } from './ValidatorNode';
export function ValidatorRing({ validatorCount, radius, sceneStateRef, }) {
    const nodes = Array.from({ length: validatorCount }, (_, i) => {
        const angle = (i / validatorCount) * Math.PI * 2;
        return (_jsx(ValidatorNode, { index: i, angle: angle, radius: radius, sceneStateRef: sceneStateRef }, i));
    });
    return _jsx("group", { children: nodes });
}
