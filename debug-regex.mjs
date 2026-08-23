const withQ = '<script type="module" src="/src/main.tsx?t=1"></script>';
console.log("A:", /<script type="module" src="/.test(withQ));
console.log("B:", /src="\/src\/main\.tsx/.test(withQ));
console.log("C:", /\/src\/main\.tsx\?[^"]*"><\/script>/.test(withQ));
const re = new RegExp('<script type="module" src="/src/main\\.tsx(\\?[^"]*)?"></script>');
console.log("D:", re.test(withQ));
