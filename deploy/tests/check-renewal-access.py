"""Read-only RAM scope check; never print credentials or DNS records."""
import argparse
import base64
import datetime
import hashlib
import hmac
import json
from pathlib import Path
import urllib.error
import urllib.parse
import urllib.request
import uuid

parser = argparse.ArgumentParser()
parser.add_argument("--credentials", type=Path, required=True)
args = parser.parse_args()
credentials = json.loads(args.credentials.read_text())


def query(domain):
    params = {
        "Action": "DescribeDomainRecords", "Version": "2015-01-09", "Format": "JSON",
        "AccessKeyId": credentials["access_key_id"], "SignatureMethod": "HMAC-SHA1",
        "SignatureVersion": "1.0", "SignatureNonce": str(uuid.uuid4()),
        "Timestamp": datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
        "DomainName": domain, "PageSize": "1",
    }
    quote = lambda value: urllib.parse.quote(value, safe="~")
    canonical = "&".join(quote(key) + "=" + quote(value) for key, value in sorted(params.items()))
    signature = hmac.new((credentials["access_key_secret"] + "&").encode(),
                         ("POST&%2F&" + quote(canonical)).encode(), hashlib.sha1).digest()
    params["Signature"] = base64.b64encode(signature).decode()
    request = urllib.request.Request("https://alidns.aliyuncs.com/",
                                     data=urllib.parse.urlencode(params).encode())
    try:
        with urllib.request.urlopen(request, timeout=20) as response:
            result = json.load(response)
    except urllib.error.HTTPError as error:
        result = json.loads(error.read())
    return result.get("Code", "OK")


zone = query("acme-pet.narendra.cn")
print("Validation subdomain read permission:", zone)
if zone != "OK":
    raise SystemExit("Validation subdomain is not accessible; credentials were not printed.")
main = query("narendra.cn")
print("Main domain read permission:", main)
if not main.startswith("Forbidden"):
    raise SystemExit("Expected explicit denial on main domain; inspect RAM policy before deployment.")
print("Scope check passed. No DNS records were changed.")
