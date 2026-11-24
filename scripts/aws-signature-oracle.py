import base64
import json
import sys
from pathlib import Path
from urllib.parse import parse_qsl, quote, urlsplit, urlunsplit

import botocore
from botocore.auth import SigV4Auth
from botocore.awsrequest import AWSRequest
from botocore.credentials import Credentials

ACCESS_KEY = 'AKIDEXAMPLE'
SECRET_KEY = 'synthetic-secret-key-for-oracle'
SESSION_TOKEN = 'synthetic/session+token='
DATE = '20261124T123456Z'


def normalize_url(url):
    parts = urlsplit(url)
    pairs = sorted((quote(k, safe='-_.~'), quote(v, safe='-_.~')) for k, v in parse_qsl(parts.query, keep_blank_values=True, encoding='utf-8', errors='strict'))
    return urlunsplit((parts.scheme, parts.netloc, parts.path, '&'.join(f'{k}={v}' for k, v in pairs), parts.fragment))


def expected(case):
    credentials = Credentials(case['accessKeyId'], case['secretAccessKey'], case.get('sessionToken'))
    request = AWSRequest(method=case['method'], url=normalize_url(case['url']), data=base64.b64decode(case['bodyBase64']), headers=case['headers'])
    request.context['timestamp'] = case['date']
    signer = SigV4Auth(credentials, case['service'], case['region'])
    signer._modify_request_before_signing(request)
    canonical = signer.canonical_request(request)
    to_sign = signer.string_to_sign(request, canonical)
    signature = signer.signature(to_sign, request)
    signer._inject_signature_to_request(request, signature)
    return {'url': request.url, 'authorization': request.headers['Authorization'], 'canonicalRequest': canonical}


def fixtures():
    import hashlib
    cases = []
    raw_cases = [
        ('form-post', 'POST', 'https://ec2.us-east-1.amazonaws.com/', 'ec2', 'us-east-1', b'Action=DescribeInstances&Version=2016-11-15', {'Content-Type': 'application/x-www-form-urlencoded'}, None),
        ('duplicate-query-encoding', 'GET', 'https://ec2.us-east-1.amazonaws.com/a//b/%2F/%E4%B8%AD?q=a+b&q=a%2Bb&empty=&q=%E4%B8%AD&z=%25&flag', 'ec2', 'us-east-1', b'', {'X-Synthetic-Whitespace': '  alpha   beta  '}, None),
        ('json-temporary-credentials', 'POST', 'https://ssm.ap-northeast-1.amazonaws.com/', 'ssm', 'ap-northeast-1', '{"Name":"synthetic/参数","WithDecryption":false}'.encode(), {'Content-Type': 'application/x-amz-json-1.1', 'X-Amz-Target': 'AmazonSSM.GetParameter'}, SESSION_TOKEN),
        ('global-iam', 'GET', 'https://iam.amazonaws.com/?Action=ListUsers&Version=2010-05-08', 'iam', 'us-east-1', b'', {'Accept': 'application/xml'}, None),
        ('empty-values-encoded-key', 'GET', 'https://logs.eu-west-1.amazonaws.com/?x=&x&%E4%B8%AD=%2F&a=%20&a=+&a=%2B', 'logs', 'eu-west-1', b'', {}, SESSION_TOKEN),
    ]
    for name, method, url, service, region, body, headers, token in raw_cases:
        headers['X-Amz-Content-SHA256'] = hashlib.sha256(body).hexdigest()
        case = {'name': name, 'method': method, 'url': url, 'service': service, 'region': region, 'bodyBase64': base64.b64encode(body).decode(), 'headers': headers, 'accessKeyId': ACCESS_KEY, 'secretAccessKey': SECRET_KEY, 'sessionToken': token, 'date': DATE}
        case['expected'] = expected(case)
        cases.append(case)
    return {'oracle': 'AWS botocore.auth.SigV4Auth', 'oracleVersion': botocore.__version__, 'reference': 'https://docs.aws.amazon.com/IAM/latest/UserGuide/reference_sigv-create-signed-request.html', 'fixtures': cases}


if __name__ == '__main__':
    if len(sys.argv) == 3 and sys.argv[1] == '--verify-wire':
        cases = sorted(Path(sys.argv[2]).glob('wire-*.json'))
        if not cases:
            raise SystemExit('No actual-wire captures found')
        for path in cases:
            case = json.loads(path.read_text())
            actual = case['headers']['authorization']
            signed = actual.split('SignedHeaders=')[1].split(',')[0].split(';')
            case['headers'] = {key: value for key, value in case['headers'].items() if key in signed}
            case['date'] = case['headers']['x-amz-date']
            oracle = expected(case)
            assert oracle['authorization'] == actual, f'{path.name}: signature mismatch'
            assert oracle['url'] == case['url'], f'{path.name}: wire URL changed'
            print(f'{path.name}: botocore verified actual wire signature')
    elif len(sys.argv) == 2 and sys.argv[1] == '--stdin':
        print(json.dumps(expected(json.load(sys.stdin))))
    else:
        output = json.dumps(fixtures(), ensure_ascii=False, indent=2) + '\n'
        if len(sys.argv) == 2:
            Path(sys.argv[1]).write_text(output)
        else:
            print(output, end='')
